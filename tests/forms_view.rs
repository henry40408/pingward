use axum_test::TestServer;
use pingward::{app, state::AppState, store::Store};

mod common;

async fn server() -> (TestServer, Store) {
    let pool = pingward::db::connect("sqlite::memory:").await.unwrap();
    pingward::db::migrate(&pool, "sqlite::memory:")
        .await
        .unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    (server, store)
}

async fn logged_in_server() -> (TestServer, Store, i64) {
    let (mut server, store) = server().await;
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
    (server, store, uid)
}

async fn server_with_project() -> (TestServer, Store, i64) {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    (server, store, pid)
}

/// Read the current session's CSRF synchronizer token straight from the DB —
/// mirrors `tests/csrf.rs::csrf_token`, needed here to authorize the
/// description-round-trip POSTs below.
async fn csrf_token(store: &Store) -> String {
    common::newest_session_csrf(&store.pool).await
}

/// The restyled channel form must keep the `.field` form-control class from
/// `assets/app.css` (Task 1) while preserving every existing input name that
/// the (unchanged) handler in `src/web.rs` depends on.
#[tokio::test]
async fn channel_form_is_restyled_and_keeps_fields() {
    let (server, _store, pid) = server_with_project().await;
    let res = server.get(&format!("/projects/{pid}/channels/new")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"field\""), "form not restyled");
    assert!(body.contains("name=\"webhook_url\""), "webhook field lost");
}

/// The restyled check form must keep the `.field` class and every field name
/// the handler in `src/web.rs` reads via `CheckForm`.
#[tokio::test]
async fn check_form_is_restyled_and_keeps_fields() {
    let (server, _store, pid) = server_with_project().await;
    let res = server.get(&format!("/projects/{pid}/checks/new")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"field\""), "form not restyled");
    for name in [
        "name",
        "description",
        "schedule_kind",
        "period_secs",
        "cron_expr",
        "grace_secs",
        "timezone",
        "scan_interval_secs",
        "max_runtime_secs",
        "nag_interval_secs",
    ] {
        assert!(
            body.contains(&format!("name=\"{name}\"")),
            "check form lost field {name}"
        );
    }
}

/// The restyled project form must keep the `.field` class and every field
/// name the handler in `src/web.rs` reads via `ProjectForm`.
#[tokio::test]
async fn project_form_is_restyled_and_keeps_fields() {
    let (server, _store, _uid) = logged_in_server().await;
    let res = server.get("/projects/new").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"field\""), "form not restyled");
    for name in [
        "name",
        "description",
        "scan_interval_secs",
        "nag_interval_secs",
    ] {
        assert!(
            body.contains(&format!("name=\"{name}\"")),
            "project form lost field {name}"
        );
    }
}

/// A description round-trips through create and edit: the create form
/// accepts a `description` field, the stored value renders (escaped) into the
/// edit form's textarea, and validation both rejects an over-length
/// description with the exact spec'd message and accepts the boundary value.
#[tokio::test]
async fn project_description_round_trips_and_is_length_validated() {
    let (server, store, uid) = logged_in_server().await;
    let token = csrf_token(&store).await;
    let res = server
        .post("/projects")
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "proj"),
            ("description", "**bold** desc"),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_see_other();
    let projects = store.list_projects_for_user(uid).await.unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].description, "**bold** desc");

    let pid = projects[0].id;
    let edit = server.get(&format!("/projects/{pid}/edit")).await;
    edit.assert_status_ok();
    assert!(
        edit.text().contains("**bold** desc"),
        "edit form must round-trip the stored description into the textarea"
    );

    // Exactly 2001 characters is rejected with the spec'd message; exactly
    // 2000 is accepted.
    let too_long = "a".repeat(2001);
    let res = server
        .post(&format!("/projects/{pid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "proj"),
            ("description", too_long.as_str()),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_ok();
    assert!(
        res.text()
            .contains("description must be at most 2000 characters"),
        "2001-char description must be rejected with the exact spec'd message"
    );
    assert_eq!(
        store.find_project(pid).await.unwrap().unwrap().description,
        "**bold** desc",
        "the rejected update must not have overwritten the stored description"
    );

    let boundary = "b".repeat(2000);
    let res = server
        .post(&format!("/projects/{pid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "proj"),
            ("description", boundary.as_str()),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_see_other();
    assert_eq!(
        store.find_project(pid).await.unwrap().unwrap().description,
        boundary,
        "a 2000-char description is exactly at the limit and must be accepted"
    );
}

/// Same round-trip + length-validation coverage as
/// `project_description_round_trips_and_is_length_validated`, for checks.
#[tokio::test]
async fn check_description_round_trips_and_is_length_validated() {
    let (server, store, pid) = server_with_project().await;
    let token = csrf_token(&store).await;
    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "backup"),
            ("description", "runs *nightly*"),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("cron_expr", ""),
            ("grace_secs", "300"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_see_other();
    let checks = store.list_checks_for_project(pid).await.unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].description, "runs *nightly*");

    let cid = checks[0].id;
    let edit = server.get(&format!("/checks/{cid}/edit")).await;
    edit.assert_status_ok();
    assert!(
        edit.text().contains("runs *nightly*"),
        "edit form must round-trip the stored description into the textarea"
    );

    let too_long = "a".repeat(2001);
    let res = server
        .post(&format!("/checks/{cid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "backup"),
            ("description", too_long.as_str()),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("cron_expr", ""),
            ("grace_secs", "300"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_ok();
    assert!(
        res.text()
            .contains("description must be at most 2000 characters"),
        "2001-char description must be rejected with the exact spec'd message"
    );

    let boundary = "b".repeat(2000);
    let res = server
        .post(&format!("/checks/{cid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "backup"),
            ("description", boundary.as_str()),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("cron_expr", ""),
            ("grace_secs", "300"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_see_other();
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().description,
        boundary,
        "a 2000-char description is exactly at the limit and must be accepted"
    );
}

/// A check created through the web form in a project that already has
/// channels comes out bound to all of them (`Store::bind_all_project_channels`,
/// called from `check_create_core`).
#[tokio::test]
async fn check_created_via_web_form_is_bound_to_existing_channels() {
    let (server, store, pid) = server_with_project().await;
    let token = csrf_token(&store).await;

    let c1 = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook1",
            r#"{"url":"http://x"}"#,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let c2 = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook2",
            r#"{"url":"http://y"}"#,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "backup"),
            ("description", ""),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("cron_expr", ""),
            ("grace_secs", "300"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_see_other();

    let checks = store.list_checks_for_project(pid).await.unwrap();
    assert_eq!(checks.len(), 1);
    let cid = checks[0].id;

    let mut bound = store.bound_channel_ids(cid).await.unwrap();
    bound.sort_unstable();
    let mut expected = vec![c1, c2];
    expected.sort_unstable();
    assert_eq!(
        bound, expected,
        "a check created in a project with existing channels must come out bound to all of them"
    );
}

/// Every credential field carries an `autocomplete` token, so a password
/// manager can fill and store them (OWASP's Authentication Cheat Sheet asks
/// applications not to make that job harder than necessary).
///
/// The tokens are not interchangeable: `current-password` on the login form is
/// what makes a manager offer the *saved* credential, while `new-password` on a
/// form that sets one is what stops it offering the same and prompts a
/// generated value instead. Getting them the wrong way round is invisible until
/// a user finds their manager unhelpful, which no other test would catch.
#[tokio::test]
async fn credential_fields_declare_their_autocomplete_role() {
    let (server, _store) = server().await;

    // Logged out, with no users: /setup is the first-run form.
    let setup = server.get("/setup").await.text();
    assert!(
        setup.contains(r#"name="username" autocomplete="username""#),
        "{setup}"
    );
    assert!(
        setup.contains(r#"type="password" autocomplete="new-password""#),
        "/setup sets a password, so it must not be tagged current-password: {setup}"
    );

    let (server, store, _uid) = logged_in_server().await;

    // /login, once a user exists — reached from a second, logged-out server on
    // the same store, since `logged_in_server`'s jar would bounce to `/`.
    let mut anon = TestServer::new(app(AppState::new(store, common::test_config())));
    anon.save_cookies();
    let login = anon.get("/login").await.text();
    assert!(
        login.contains(r#"name="username" autocomplete="username""#),
        "{login}"
    );
    assert!(
        login.contains(r#"type="password" autocomplete="current-password""#),
        "/login submits an existing credential: {login}"
    );

    // /admin manages *other* people's accounts, so its username field opts out
    // of autofill entirely — offering the signed-in admin's own username there
    // is never right — and both password fields set a new credential.
    let admin = server.get("/admin").await.text();
    assert!(
        admin.contains(r#"name="username" autocomplete="off""#),
        "{admin}"
    );
    assert_eq!(
        admin.matches(r#"autocomplete="new-password""#).count(),
        2,
        "both the reset field and the add-user field must be new-password: {admin}"
    );

    // /account already had these; pinned here so the set stays complete.
    let account = server.get("/account").await.text();
    assert!(
        account.contains(r#"autocomplete="current-password""#),
        "{account}"
    );
    assert_eq!(account.matches(r#"autocomplete="new-password""#).count(), 2);
}

// --- duration suggestion lists ---------------------------------------------
//
// Every duration-valued field in the UI carries a `<datalist>` so the unit
// suffixes its help text mentions are visible without being read. The list is
// a hint and never a constraint, so these tests assert two separate things:
// that the markup wires the fields to the list at all, and — the part that
// actually matters — that everything the list offers is a value the handler
// behind the field accepts. A suggestion the form would reject is worse than
// no suggestion, because the user picks it out of the browser's own dropdown.

/// The opening `<input …>` tag carrying `id="{id}"`, as raw markup.
fn input_tag<'a>(body: &'a str, id: &str) -> &'a str {
    body.split('<')
        .find(|tag| tag.starts_with("input") && tag.contains(&format!("id=\"{id}\"")))
        .unwrap_or_else(|| panic!("no <input id=\"{id}\"> on the page"))
}

/// Assert `body` renders exactly one `<datalist id="{id}">`, offering an
/// `<option>` for every entry in `want`.
fn assert_list(body: &str, id: &str, want: &[&str]) {
    // Non-vacuity: an empty list would satisfy every assertion below.
    assert!(!want.is_empty(), "the suggestion list itself is empty");
    assert_eq!(
        body.matches(&format!("<datalist id=\"{id}\">")).count(),
        1,
        "expected exactly one <datalist id=\"{id}\"> — a duplicate id leaves the \
         second one as dead markup"
    );
    for value in want {
        assert!(
            body.contains(&format!("<option value=\"{value}\">")),
            "list {id} is missing the suggestion {value:?}"
        );
    }
}

/// Assert each named field is wired to `list="{id}"`.
fn assert_wired(body: &str, id: &str, fields: &[&str]) {
    for field in fields {
        let tag = input_tag(body, field);
        assert!(
            tag.contains(&format!("list=\"{id}\"")),
            "field {field} is not wired to the {id} suggestions: {tag}"
        );
    }
}

/// The check form's five duration fields all point at one shared list.
#[tokio::test]
async fn check_form_duration_fields_offer_the_shared_suggestions() {
    let (server, _store, pid) = server_with_project().await;
    let body = server
        .get(&format!("/projects/{pid}/checks/new"))
        .await
        .text();
    assert_list(&body, "dur-list", pingward::view::durations());
    assert_wired(
        &body,
        "dur-list",
        &[
            "period_secs",
            "grace_secs",
            "scan_interval_secs",
            "max_runtime_secs",
            "nag_interval_secs",
        ],
    );
}

/// The project form's two overrides point at the same list, built from the
/// same source, so the two forms cannot drift apart.
#[tokio::test]
async fn project_form_duration_fields_offer_the_shared_suggestions() {
    let (server, _store, _uid) = logged_in_server().await;
    let body = server.get("/projects/new").await.text();
    assert_list(&body, "dur-list", pingward::view::durations());
    assert_wired(
        &body,
        "dur-list",
        &["scan_interval_secs", "nag_interval_secs"],
    );
}

/// `/admin`'s two global intervals are duration fields and get the list. The
/// retention fields beside them are a count of days, handled by a different
/// branch of `settings_save` (`SettingKind::Days`), so offering them `5m`
/// would be offering a value the save rejects.
#[tokio::test]
async fn admin_settings_duration_fields_offer_the_shared_suggestions() {
    let (server, _store, _uid) = logged_in_server().await;
    let body = server.get("/admin").await.text();
    assert_list(&body, "dur-list", pingward::view::durations());
    assert_wired(&body, "dur-list", &["scan_interval", "nag_interval"]);
    for days_field in [
        "pings_retention_days",
        "notifications_retention_days",
        "audit_retention_days",
    ] {
        let tag = input_tag(&body, days_field);
        assert!(
            !tag.contains("list=\"dur-list\""),
            "{days_field} counts days, not durations, and must not offer duration \
             suggestions: {tag}"
        );
    }
}

/// The API key expiry is a duration field on a different scale, so it gets its
/// own list rather than the shared one.
#[tokio::test]
async fn api_key_expiry_offers_its_own_suggestions() {
    let (server, _store, _uid) = logged_in_server().await;
    let body = server.get("/account").await.text();
    assert_list(&body, "expiry-list", pingward::view::expiries());
    assert_wired(&body, "expiry-list", &["expires_in"]);
    assert!(
        !body.contains("<datalist id=\"dur-list\">"),
        "/account has no interval fields, so the shared list has no business there"
    );
}

/// The point of the whole change: every value the browser offers is one the
/// form actually stores. Each suggestion is submitted into all five of the
/// check form's duration fields at once and read back, so a future edit to
/// `view::durations` adding something `parse_duration` cannot handle fails
/// here rather than in a user's face.
#[tokio::test]
async fn every_suggested_duration_is_accepted_by_the_check_form() {
    let (server, store, pid) = server_with_project().await;
    let token = csrf_token(&store).await;
    for suggestion in pingward::view::durations() {
        let res = server
            .post(&format!("/projects/{pid}/checks"))
            .form(&[
                ("_csrf", token.as_str()),
                ("name", suggestion),
                ("description", ""),
                ("schedule_kind", "period"),
                ("period_secs", suggestion),
                ("cron_expr", ""),
                ("grace_secs", suggestion),
                ("timezone", "UTC"),
                ("scan_interval_secs", suggestion),
                ("max_runtime_secs", suggestion),
                ("nag_interval_secs", suggestion),
            ])
            .await;
        assert_eq!(
            res.status_code(),
            303,
            "the check form rejected its own suggestion {suggestion:?}"
        );
        let want = pingward::duration::parse_duration(suggestion).unwrap();
        let checks = store.list_checks_for_project(pid).await.unwrap();
        let stored = checks
            .iter()
            .find(|c| c.name == *suggestion)
            .unwrap_or_else(|| panic!("no check stored for {suggestion:?}"));
        assert_eq!(
            stored.period_secs,
            Some(want),
            "{suggestion:?} did not round-trip into the stored period"
        );
        assert_eq!(stored.grace_secs, want);
        assert_eq!(stored.scan_interval_secs, Some(want));
        assert_eq!(stored.max_runtime_secs, Some(want));
        assert_eq!(stored.nag_interval_secs, Some(want));
    }
}
