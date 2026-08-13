//! What still works with JavaScript switched off.
//!
//! `app.js` is progressive enhancement everywhere *except* where these tests
//! are pointed. Several things had quietly stopped being optional:
//!
//! 1. A check row's only route to the check page was the delegated `data-href`
//!    click handler, so with no JS the dashboard led nowhere at all.
//! 2. The history sections' pager and Clear controls are real `<a href>`s
//!    aimed at fragment endpoints, which answered a plain navigation with a
//!    bare partial — no `<head>`, so no stylesheet, no nav, no way back.
//! 3. Every irreversible action asked "are you sure?" through a `data-confirm`
//!    attribute that only `app.js` reads, so with no JS a misclick deleted a
//!    project outright.
//! 4. The filter forms had no method, no action, no field names and a
//!    `type="button"` submit — four reasons one click did nothing.
//! 5. `/admin`'s heartbeat tiles rendered an empty div where the age goes.
//!
//! Every one is invisible to the browser suite, which runs with JS on — except
//! for the `no-js` Playwright project (`e2e/features/no_js.feature`), which
//! covers the parts that are CSS or navigation rather than markup. These
//! assertions are the server-side half of the same guard.

use axum::http::StatusCode;
use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};

mod common;

async fn logged_in_server() -> (TestServer, Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "admin"),
            ("password", "pw"),
        ])
        .await;
    // Destructive POSTs below go through `csrf_guard` like any other; send the
    // session's token as a default header so a rejection here can only mean the
    // confirmation gate.
    let tok = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", tok.as_str());
    (server, store, uid)
}

async fn check_for(store: &Store, owner: i64, uuid: &str) -> (i64, i64) {
    let pid = store
        .create_project(owner, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            ping_uuid: uuid,
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    (pid, cid)
}

// --- rows reach their page without a click handler ---

/// The dashboard's whole job is getting you to a check. The row is a `div`
/// (a flex container three templates share), so the link has to be inside it.
#[tokio::test]
async fn dashboard_check_rows_carry_a_real_link() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let body = server.get("/").await.text();
    assert!(
        body.contains(&format!("href=\"/checks/{cid}\"")),
        "dashboard row has no anchor to its check: {body}"
    );
}

#[tokio::test]
async fn project_check_rows_carry_a_real_link() {
    let (server, store, uid) = logged_in_server().await;
    let (pid, cid) = check_for(&store, uid, "cu").await;

    let body = server.get(&format!("/projects/{pid}")).await.text();
    assert!(
        body.contains(&format!("href=\"/checks/{cid}\"")),
        "project row has no anchor to its check: {body}"
    );
}

#[tokio::test]
async fn admin_project_rows_carry_a_real_link() {
    let (server, store, uid) = logged_in_server().await;
    let (pid, _cid) = check_for(&store, uid, "cu").await;

    let body = server.get("/admin").await.text();
    assert!(
        body.contains(&format!("href=\"/admin/projects/{pid}\"")),
        "admin project row has no anchor to its project: {body}"
    );
}

/// The row must not go back to simulating a link with ARIA: `role="link"` plus
/// `tabindex` buys a focus ring and Enter, and still leaves the row dead with
/// JS off — which is exactly the state this file exists to prevent.
#[tokio::test]
async fn rows_do_not_simulate_a_link_with_aria() {
    let (server, store, uid) = logged_in_server().await;
    check_for(&store, uid, "cu").await;

    for path in ["/", "/admin"] {
        let body = server.get(path).await.text();
        assert!(
            !body.contains("role=\"link\""),
            "{path} still fakes a link instead of rendering one: {body}"
        );
    }
}

// --- fragment endpoints answer a navigation with a page ---

#[tokio::test]
async fn pings_fragment_redirects_a_real_navigation_to_the_check_page() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let res = server.get(&format!("/checks/{cid}/pings")).await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        format!("/checks/{cid}#pings-section")
    );
}

/// The pager cursor and the active filter live entirely in the query string,
/// and the full check page parses the same `CheckPageQuery` — so carrying it
/// across is what makes an unscripted "Older →" actually page.
#[tokio::test]
async fn the_redirect_carries_the_cursor_and_filter() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let res = server
        .get(&format!("/checks/{cid}/pings?pb=42&pk=fail"))
        .await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        format!("/checks/{cid}?pb=42&pk=fail#pings-section")
    );
}

#[tokio::test]
async fn notifications_fragment_redirects_a_real_navigation() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let res = server.get(&format!("/checks/{cid}/notifications")).await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        format!("/checks/{cid}#notifs-section")
    );
}

#[tokio::test]
async fn admin_check_fragments_redirect_within_the_admin_prefix() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let res = server.get(&format!("/admin/checks/{cid}/pings")).await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        format!("/admin/checks/{cid}#pings-section")
    );

    let res = server
        .get(&format!("/admin/checks/{cid}/notifications"))
        .await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        format!("/admin/checks/{cid}#notifs-section")
    );
}

#[tokio::test]
async fn audit_fragment_redirects_a_real_navigation_to_admin() {
    let (server, store, uid) = logged_in_server().await;
    check_for(&store, uid, "cu").await;

    let res = server.get("/admin/audit?aaction=admin.access").await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        res.header("location"),
        "/admin?aaction=admin.access#audit-section"
    );
}

/// The redirect is presentation only: `app.js` still gets its partial, which
/// is what keeps the in-place swap a swap.
#[tokio::test]
async fn a_fetch_caller_still_gets_the_bare_fragment() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let res = server
        .get(&format!("/checks/{cid}/pings"))
        .add_header("x-requested-with", "fetch")
        .await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("data-testid=\"pings-filters\""),
        "fetch caller did not get the fragment: {body}"
    );
    assert!(
        !body.contains("<title>"),
        "fetch caller got a whole page, not a fragment: {body}"
    );
}

/// Ownership is resolved *before* the redirect decision, so the fallback
/// cannot become a cheap way to confirm that someone else's check exists.
#[tokio::test]
async fn the_redirect_never_answers_for_another_users_check() {
    let (server, store, _uid) = logged_in_server().await;
    let other = store
        .create_user("other", Some("x"), false, chrono::Utc::now())
        .await
        .unwrap();
    let (_pid, cid) = check_for(&store, other, "cu2").await;

    server
        .get(&format!("/checks/{cid}/pings"))
        .await
        .assert_status_not_found();
    server
        .get(&format!("/checks/{cid}/notifications"))
        .await
        .assert_status_not_found();
}

// --- the scheduler heartbeat states its own age ------------------------------

/// `/admin`'s heartbeat tiles rendered an empty `.hb-ago` div for `app.js` to
/// fill in every second, so with no script the one number an operator actually
/// reads off them — how long ago the loop last ran — was simply blank.
#[tokio::test]
async fn the_scheduler_heartbeat_renders_its_age_server_side() {
    let (server, store, _uid) = logged_in_server().await;
    let ninety_minutes_ago = chrono::Utc::now() - chrono::Duration::minutes(90);
    store
        .set_setting("last_scan_at", &ninety_minutes_ago.to_rfc3339())
        .await
        .unwrap();

    let body = server.get("/admin").await.text();
    assert!(
        body.contains("class=\"hb-ago\" data-ago=") && body.contains("1h ago"),
        "the scan heartbeat has no server-rendered age: {body}"
    );
}

/// An unparseable stamp renders no age rather than a wrong one — the absolute
/// timestamp beside it still shows either way.
#[tokio::test]
async fn an_unparseable_heartbeat_stamp_renders_no_age() {
    let (server, store, _uid) = logged_in_server().await;
    store
        .set_setting("last_scan_at", "not-a-date")
        .await
        .unwrap();

    let body = server.get("/admin").await.text();
    assert!(
        body.contains("class=\"hb-ago\" data-ago=\"not-a-date\"></div>"),
        "an unparseable stamp should leave the age empty: {body}"
    );
}

// --- the light palette exists twice, and must stay identical -----------------

/// Every `--token: value` pair inside the first `{ … }` block following
/// `marker`. Brace-counting is overkill here: neither block nests.
fn palette_after(css: &str, marker: &str) -> Vec<(String, String)> {
    let start = css
        .find(marker)
        .unwrap_or_else(|| panic!("{marker} is gone from app.css"))
        + marker.len();
    let rest = &css[start..];
    let end = rest.find('}').expect("unterminated block");
    rest[..end]
        .lines()
        .filter_map(|l| l.trim().strip_prefix("--"))
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| {
            (
                k.trim().to_string(),
                v.trim().trim_end_matches(';').to_string(),
            )
        })
        .collect()
}

/// The light palette is written twice — once for the `data-theme` attribute
/// `theme-init.js` sets, once inside a `prefers-color-scheme` query for a
/// browser that never ran it. A selector list cannot span a media query, so
/// the duplication is unavoidable; this is what stops the copies drifting.
/// A token added or retuned in one block only is invisible until someone opens
/// the app with script off, which is precisely the case nobody tests by hand.
#[test]
fn the_two_light_palettes_are_identical() {
    let css = include_str!("../assets/app.css");
    let scripted = palette_after(css, ":root[data-theme=\"light\"] {");
    let scriptless = palette_after(css, ":root:not([data-theme]) {");

    assert!(
        scripted.len() > 20,
        "parsed only {} tokens — the marker probably moved and this test is \
         passing vacuously",
        scripted.len()
    );
    assert_eq!(
        scripted, scriptless,
        "the scripted and scriptless light palettes have drifted; every token \
         must appear in both, with the same value and in the same order"
    );
}

// --- the filter forms submit on their own ------------------------------------

/// A GET submission replaces the whole query string, so the pings form has to
/// re-send the notifications filter as hidden state or narrowing one section
/// would silently clear the other. Asserted on the rendered page rather than
/// through a browser because it is the server that has to emit them.
#[tokio::test]
async fn each_filter_form_carries_the_other_sections_filter() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let body = server
        .get(&format!("/checks/{cid}?pk=fail&ne=down"))
        .await
        .text();

    // The pings form re-sends the notifications half...
    assert!(
        body.contains("<input type=\"hidden\" name=\"ne\" value=\"down\">"),
        "pings form drops the notifications filter: {body}"
    );
    // ...and the notifications form re-sends the pings half.
    assert!(
        body.contains("<input type=\"hidden\" name=\"pk\" value=\"fail\">"),
        "notifications form drops the pings filter: {body}"
    );
    // Neither re-sends its own keys as hidden state — the visible controls
    // carry those, and a duplicate would submit the stale value alongside.
    assert!(
        !body.contains("<input type=\"hidden\" name=\"pk\" value=\"fail\">\n  <input"),
        "a form re-sent its own filter as hidden state: {body}"
    );
    // Clear drops only its own section's filter.
    assert!(
        body.contains(&format!("/checks/{cid}/pings?ne=down")),
        "the pings Clear link drops the notifications filter: {body}"
    );
    assert!(
        body.contains(&format!("/checks/{cid}/notifications?pk=fail")),
        "the notifications Clear link drops the pings filter: {body}"
    );
}

/// The forms post to the page, not to the fragment endpoint, and the button is
/// a real submit — four separate reasons a scriptless click used to do nothing.
#[tokio::test]
async fn the_filter_forms_are_real_get_forms() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let body = server.get(&format!("/checks/{cid}")).await.text();
    assert!(
        body.contains(&format!("method=\"get\" action=\"/checks/{cid}\"")),
        "a filter form has no GET action: {body}"
    );
    assert!(
        !body.contains("type=\"button\" data-apply"),
        "a Filter button is still type=button, so it submits nothing: {body}"
    );
    for name in ["pk", "pfrom", "pto", "ne", "ns", "nfrom", "nto"] {
        assert!(
            body.contains(&format!("name=\"{name}\"")),
            "filter control {name} has no name, so it submits nothing: {body}"
        );
    }
}

// --- irreversible actions ask before they run --------------------------------
//
// With JS the question is a native `confirm()` driven by the form's
// `data-confirm` attribute; the server never sees that happen and must not
// assume it did. Every one of these actions therefore runs only when the
// request carries `?confirmed=1`, and otherwise answers with the same question
// as a page. Each test asserts *both* halves — that the unconfirmed POST
// changed nothing, and that the confirmed one went through — because a gate
// that refused everything would satisfy either half on its own.

/// The interstitial, identified by the button that goes through with it.
fn is_confirmation_page(body: &str) -> bool {
    body.contains("data-testid=\"confirm-submit\"")
}

#[tokio::test]
async fn deleting_a_check_asks_first_and_deletes_only_when_confirmed() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;

    let res = server.post(&format!("/checks/{cid}/delete")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(is_confirmation_page(&body), "no confirmation page: {body}");
    // The page's own form re-posts the same action, now confirmed.
    assert!(
        body.contains(&format!("action=\"/checks/{cid}/delete?confirmed=1\"")),
        "the page does not offer to complete the action: {body}"
    );
    assert!(
        store.find_check(cid).await.unwrap().is_some(),
        "the check was deleted before anyone confirmed"
    );

    server
        .post(&format!("/checks/{cid}/delete?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(
        store.find_check(cid).await.unwrap().is_none(),
        "the confirmed delete did not go through"
    );
}

#[tokio::test]
async fn deleting_a_project_asks_first() {
    let (server, store, uid) = logged_in_server().await;
    let (pid, _cid) = check_for(&store, uid, "cu").await;

    let res = server.post(&format!("/projects/{pid}/delete")).await;
    res.assert_status_ok();
    assert!(is_confirmation_page(&res.text()));
    assert!(
        store.find_project(pid).await.unwrap().is_some(),
        "the project was deleted before anyone confirmed"
    );

    server
        .post(&format!("/projects/{pid}/delete?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.find_project(pid).await.unwrap().is_none());
}

/// Regenerating is not a delete, but it breaks every job still pinging the old
/// URL — the same "cannot be undone by clicking again" shape.
#[tokio::test]
async fn regenerating_a_ping_url_asks_first() {
    let (server, store, uid) = logged_in_server().await;
    let (_pid, cid) = check_for(&store, uid, "cu").await;
    let before = store.find_check(cid).await.unwrap().unwrap().ping_uuid;

    let res = server.post(&format!("/checks/{cid}/regenerate")).await;
    res.assert_status_ok();
    assert!(is_confirmation_page(&res.text()));
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().ping_uuid,
        before,
        "the ping URL changed before anyone confirmed"
    );

    server
        .post(&format!("/checks/{cid}/regenerate?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert_ne!(
        store.find_check(cid).await.unwrap().unwrap().ping_uuid,
        before
    );
}

#[tokio::test]
async fn deleting_a_user_asks_first() {
    let (server, store, _uid) = logged_in_server().await;
    let victim = store
        .create_user("victim", Some("x"), false, chrono::Utc::now())
        .await
        .unwrap();

    let res = server.post(&format!("/admin/users/{victim}/delete")).await;
    res.assert_status_ok();
    assert!(is_confirmation_page(&res.text()));
    assert!(
        store.find_user_by_id(victim).await.unwrap().is_some(),
        "the user was deleted before anyone confirmed"
    );

    server
        .post(&format!("/admin/users/{victim}/delete?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(victim).await.unwrap().is_none());
}

/// The two toggles are gated in one direction only, matching what the template
/// renders `data-confirm` for. Taking access away asks; handing it back does
/// not, so an operator undoing a mistake is not made to confirm the undo.
#[tokio::test]
async fn the_user_toggles_ask_only_in_the_direction_that_takes_access_away() {
    let (server, store, _uid) = logged_in_server().await;
    let member = store
        .create_user("member", Some("x"), false, chrono::Utc::now())
        .await
        .unwrap();
    let disabled = async |id: i64| store.find_user_by_id(id).await.unwrap().unwrap().disabled;
    let is_admin = async |id: i64| store.find_user_by_id(id).await.unwrap().unwrap().is_admin;

    // Disabling asks; the account stays enabled until confirmed.
    let res = server
        .post(&format!("/admin/users/{member}/disabled"))
        .await;
    res.assert_status_ok();
    assert!(is_confirmation_page(&res.text()));
    assert!(!disabled(member).await, "disabled without confirming");
    server
        .post(&format!("/admin/users/{member}/disabled?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(disabled(member).await);

    // Re-enabling does not ask at all.
    server
        .post(&format!("/admin/users/{member}/disabled"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(!disabled(member).await);

    // Promoting does not ask for a confirmation either — it is gated by the
    // elevation check instead, which is a password and a stronger question than
    // "are you sure?". Unlock first, or the redirect below is the elevation
    // bounce rather than a completed promotion.
    server
        .post("/admin/unlock")
        .form(&[("password", "pw")])
        .await
        .assert_status(StatusCode::SEE_OTHER);
    server
        .post(&format!("/admin/users/{member}/admin"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(is_admin(member).await, "promotion did not go through");

    // Demoting asks.
    let res = server.post(&format!("/admin/users/{member}/admin")).await;
    res.assert_status_ok();
    assert!(is_confirmation_page(&res.text()));
    assert!(is_admin(member).await, "demoted without confirming");
}

#[tokio::test]
async fn revoking_an_api_key_asks_first() {
    let (server, store, uid) = logged_in_server().await;
    let kid = store
        .insert_api_key(uid, "ci", "hash", "pw_abcd", None, chrono::Utc::now())
        .await
        .unwrap();

    let res = server
        .post(&format!("/account/api-keys/{kid}/delete"))
        .await;
    res.assert_status_ok();
    assert!(is_confirmation_page(&res.text()));
    assert_eq!(
        store.list_api_keys_for_user(uid).await.unwrap().len(),
        1,
        "the key was revoked before anyone confirmed"
    );

    server
        .post(&format!("/account/api-keys/{kid}/delete?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.list_api_keys_for_user(uid).await.unwrap().is_empty());
}

/// Signing a browser out is not undoable from that browser, so it asks too —
/// including the "revoke every other session" bulk control, which is the one
/// most likely to be hit by mistake.
#[tokio::test]
async fn revoking_sessions_asks_first() {
    let (server, store, uid) = logged_in_server().await;
    let sessions = store
        .list_sessions_for_user(uid, chrono::Utc::now())
        .await
        .unwrap();
    let handle = pingward::apikey::hash_api_key(&sessions[0].id);

    let res = server.post("/account/sessions/revoke-others").await;
    res.assert_status_ok();
    assert!(is_confirmation_page(&res.text()));

    let res = server
        .post(&format!("/account/sessions/{handle}/revoke"))
        .await;
    res.assert_status_ok();
    assert!(is_confirmation_page(&res.text()));
    assert_eq!(
        store
            .list_sessions_for_user(uid, chrono::Utc::now())
            .await
            .unwrap()
            .len(),
        1,
        "the session was revoked before anyone confirmed"
    );

    // Confirming revokes the current session, which signs this browser out.
    server
        .post(&format!("/account/sessions/{handle}/revoke?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(
        store
            .list_sessions_for_user(uid, chrono::Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
}

/// The `/admin` twins share the owner templates but not the handlers, so each
/// needs its own gate — and each needs its own assertion that it has one.
#[tokio::test]
async fn the_admin_twins_ask_first_too() {
    let (server, store, _uid) = logged_in_server().await;
    let owner = store
        .create_user("owner", Some("x"), false, chrono::Utc::now())
        .await
        .unwrap();
    let (pid, cid) = check_for(&store, owner, "cu2").await;
    let before = store.find_check(cid).await.unwrap().unwrap().ping_uuid;

    let res = server
        .post(&format!("/admin/checks/{cid}/regenerate"))
        .await;
    res.assert_status_ok();
    assert!(is_confirmation_page(&res.text()));
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().ping_uuid,
        before
    );

    let res = server.post(&format!("/admin/checks/{cid}/delete")).await;
    res.assert_status_ok();
    assert!(is_confirmation_page(&res.text()));
    assert!(store.find_check(cid).await.unwrap().is_some());

    let res = server.post(&format!("/admin/projects/{pid}/delete")).await;
    res.assert_status_ok();
    assert!(is_confirmation_page(&res.text()));
    assert!(store.find_project(pid).await.unwrap().is_some());

    // ...and confirming each still works, deepest first.
    server
        .post(&format!("/admin/checks/{cid}/regenerate?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert_ne!(
        store.find_check(cid).await.unwrap().unwrap().ping_uuid,
        before
    );
    server
        .post(&format!("/admin/checks/{cid}/delete?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.find_check(cid).await.unwrap().is_none());
    server
        .post(&format!("/admin/projects/{pid}/delete?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.find_project(pid).await.unwrap().is_none());
}

/// Authorization still comes first: a stranger's resource is a 404, not an
/// invitation to confirm deleting something they cannot see.
#[tokio::test]
async fn the_confirmation_never_answers_for_another_users_resource() {
    let (server, store, _uid) = logged_in_server().await;
    let other = store
        .create_user("other", Some("x"), false, chrono::Utc::now())
        .await
        .unwrap();
    let (pid, cid) = check_for(&store, other, "cu2").await;

    server
        .post(&format!("/checks/{cid}/delete"))
        .await
        .assert_status_not_found();
    server
        .post(&format!("/projects/{pid}/delete"))
        .await
        .assert_status_not_found();
}
