//! The elevation gate on `/admin`'s access-granting actions
//! (`web::elevation` / `src/elevate.rs`).
//!
//! `/admin`'s controls are single-button inline forms in a table row —
//! `users_toggle_admin` posts no body at all — so the per-action password field
//! `/account` uses does not fit. Re-authentication is decoupled instead: a
//! refused action bounces to `/admin/unlock`, an interstitial that *explains*
//! the requirement and takes the password; confirming lasts a while.
//!
//! The line drawn is **granting versus removing access**. Creating a user,
//! resetting a password and promoting to admin each hand out access that
//! outlives the browser session that did it. Disabling, demoting and deleting
//! take access away, and an operator who thinks they are under attack must be
//! able to do those without first finding their password — so those are
//! deliberately ungated, and this file pins both halves.

use axum::http::StatusCode;
use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};

mod common;

const ADMIN_PW: &str = "pw";

/// A signed-in admin, **not** unlocked, plus a second ordinary account to act
/// on. Returns the admin's server and the target's id.
async fn locked_admin() -> (TestServer, Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let phc = pingward::auth::hash_password(ADMIN_PW).unwrap();
    store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    let target = store
        .create_user("dave", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();

    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "admin"),
            ("password", ADMIN_PW),
        ])
        .await;
    let tok = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", tok.as_str());
    (server, store, target)
}

// --- granting access is gated ---

#[tokio::test]
async fn creating_a_user_is_refused_while_locked() {
    let (server, store, _dave) = locked_admin().await;
    server
        .post("/admin/users")
        .form(&[("username", "carol"), ("password", "a long enough phrase")])
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(
        store
            .find_user_by_username("carol")
            .await
            .unwrap()
            .is_none(),
        "a locked admin must not have created an account"
    );
    // The refusal is an interstitial, not a silent bounce: the admin lands on
    // the page that explains the requirement, with what was refused named.
    let bounced = server.get("/admin/unlock").await.text();
    assert!(bounced.contains("unlock-bounced"), "{bounced}");
    assert!(bounced.contains("unlock-input"), "{bounced}");
}

#[tokio::test]
async fn resetting_a_password_is_refused_while_locked() {
    let (server, store, dave) = locked_admin().await;
    let before = store
        .find_user_by_id(dave)
        .await
        .unwrap()
        .unwrap()
        .password_hash
        .unwrap();
    server
        .post(&format!("/admin/users/{dave}/password"))
        .form(&[("password", "a long enough phrase")])
        .await
        .assert_status(StatusCode::SEE_OTHER);
    let after = store
        .find_user_by_id(dave)
        .await
        .unwrap()
        .unwrap()
        .password_hash
        .unwrap();
    assert_eq!(before, after, "the credential must be untouched");
}

#[tokio::test]
async fn promoting_to_admin_is_refused_while_locked() {
    let (server, store, dave) = locked_admin().await;
    server
        .post(&format!("/admin/users/{dave}/admin"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(!store.find_user_by_id(dave).await.unwrap().unwrap().is_admin);
}

// --- removing access is not gated ---

/// Demoting is the *other* direction through the same handler, and must stay
/// available while locked: revoking someone's admin rights is what an operator
/// reaches for when they think an account is compromised.
#[tokio::test]
async fn demoting_an_admin_works_while_locked() {
    let (server, store, dave) = locked_admin().await;
    store.set_user_admin(dave, true).await.unwrap();
    server
        .post(&format!("/admin/users/{dave}/admin"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(!store.find_user_by_id(dave).await.unwrap().unwrap().is_admin);
}

#[tokio::test]
async fn disabling_and_deleting_work_while_locked() {
    let (server, store, dave) = locked_admin().await;
    server
        .post(&format!("/admin/users/{dave}/disabled"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(dave).await.unwrap().unwrap().disabled);

    server
        .post(&format!("/admin/users/{dave}/delete"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(dave).await.unwrap().is_none());
}

// --- unlocking ---

#[tokio::test]
async fn unlocking_then_granting_works() {
    let (server, store, dave) = locked_admin().await;
    common::unlock_admin(&server, ADMIN_PW).await;

    let body = server.get("/admin").await.text();
    assert!(body.contains("elevation-state"), "{body}");
    assert!(body.contains("stay available for another"), "{body}");
    // And the page itself reports the live window rather than re-asking.
    let page = server.get("/admin/unlock").await.text();
    assert!(page.contains("unlock-state"), "{page}");
    assert!(!page.contains("unlock-input"), "{page}");

    server
        .post(&format!("/admin/users/{dave}/admin"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(dave).await.unwrap().unwrap().is_admin);
}

#[tokio::test]
async fn the_wrong_password_does_not_unlock() {
    let (server, store, dave) = locked_admin().await;
    let res = server
        .post("/admin/unlock")
        .form(&[("password", "not-the-password")])
        .await;
    // The unlock page re-renders with the error, keeping the explanation in
    // front of an admin who is mid-flow.
    res.assert_status_ok();
    assert!(res.text().contains("That password is not correct."));
    assert!(res.text().contains("unlock-input"));

    server.post(&format!("/admin/users/{dave}/admin")).await;
    assert!(
        !store.find_user_by_id(dave).await.unwrap().unwrap().is_admin,
        "a failed unlock must not have elevated anything"
    );
}

/// The unlock form is reachable from an authenticated seat, so it would be a
/// third password oracle if it were not metered. It shares the account limiter
/// with the login form and `/account`.
#[tokio::test]
async fn repeated_wrong_unlocks_exhaust_the_account_budget() {
    let (server, _store, _dave) = locked_admin().await;
    for _ in 0..pingward::ratelimit::ACCOUNT_MAX_ATTEMPTS {
        server
            .post("/admin/unlock")
            .form(&[("password", "not-the-password")])
            .await
            .assert_status_ok();
    }
    let res = server
        .post("/admin/unlock")
        .form(&[("password", ADMIN_PW)])
        .await;
    assert!(res.text().contains("Too many attempts"), "{}", res.text());
}

/// Elevation belongs to a session, not to a person: unlocking in one browser
/// must not unlock another that is signed in as the same admin.
#[tokio::test]
async fn elevation_does_not_leak_to_another_session() {
    let (server, store, dave) = locked_admin().await;
    common::unlock_admin(&server, ADMIN_PW).await;

    // A second browser on the same store, signed in as the same admin.
    let state = AppState::new(store.clone(), common::test_config());
    let mut other = TestServer::new(app(state));
    other.save_cookies();
    let csrf = common::anonymous_csrf(&mut other).await;
    other
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "admin"),
            ("password", ADMIN_PW),
        ])
        .await;
    let tok = common::newest_session_csrf(&store.pool).await;
    other.add_header("x-csrf-token", tok.as_str());

    other.post(&format!("/admin/users/{dave}/admin")).await;
    assert!(
        !store.find_user_by_id(dave).await.unwrap().unwrap().is_admin,
        "the second session was never unlocked"
    );
}

/// Signing out and back in starts locked again.
///
/// Note what this does and does not prove: a new session gets a new id and
/// therefore a new handle, so it would be locked even if `logout` forgot to
/// call `Elevations::revoke`. The revoke itself is unit-tested
/// (`elevate::tests::revoke_ends_the_window_immediately`); what matters here is
/// the user-visible half — elevation never survives a sign-out.
#[tokio::test]
async fn signing_out_and_back_in_starts_locked() {
    let (server, store, dave) = locked_admin().await;
    common::unlock_admin(&server, ADMIN_PW).await;
    server.post("/logout").await;

    let state = AppState::new(store.clone(), common::test_config());
    let mut again = TestServer::new(app(state));
    again.save_cookies();
    let csrf = common::anonymous_csrf(&mut again).await;
    again
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "admin"),
            ("password", ADMIN_PW),
        ])
        .await;
    let tok = common::newest_session_csrf(&store.pool).await;
    again.add_header("x-csrf-token", tok.as_str());

    again.post(&format!("/admin/users/{dave}/admin")).await;
    assert!(
        !store.find_user_by_id(dave).await.unwrap().unwrap().is_admin,
        "a fresh session after logout must start locked"
    );
}

// --- the interstitial page ---

/// The page has to *explain*, not just ask. An admin who is already signed in
/// and gets asked for their password again will otherwise reasonably wonder
/// whether something is wrong.
#[tokio::test]
async fn the_unlock_page_explains_the_requirement() {
    let (server, _store, _dave) = locked_admin().await;
    let body = server.get("/admin/unlock").await.text();

    assert!(body.contains("unlock-input"), "{body}");
    // What it covers, and — just as important — what it does not. The three
    // actions are named inline in the prose (emphasised with `<strong>`, which
    // `.crumb strong` lifts out of the muted body colour) rather than as
    // badges: `.badge` is the check-status vocabulary and reads as a status
    // pill wherever it appears.
    assert!(body.contains("unlock-gated"), "{body}");
    assert!(
        body.contains("<strong>granting admin rights</strong>"),
        "{body}"
    );
    assert!(!body.contains("badge"), "{body}");
    assert!(body.contains("disabling, demoting, deleting"), "{body}");
    // It is the same password again. Calling this a second *factor* would be
    // wrong, and an admin hunting for a TOTP app is a support ticket.
    assert!(body.contains("not a second factor"), "{body}");
    // The window is rendered from the constant rather than written into the
    // copy, so the two cannot drift.
    assert!(body.contains("15m"), "{body}");
    // A way out that is not the browser back button.
    assert!(body.contains("unlock-cancel"), "{body}");
}

/// Arriving under one's own steam is not the same as being bounced here: only
/// the bounce names a refused action, and the notice is one-shot.
#[tokio::test]
async fn the_bounce_notice_is_one_shot_and_absent_when_navigating() {
    let (server, _store, dave) = locked_admin().await;

    assert!(
        !server
            .get("/admin/unlock")
            .await
            .text()
            .contains("unlock-bounced")
    );

    server.post(&format!("/admin/users/{dave}/admin")).await;
    assert!(
        server
            .get("/admin/unlock")
            .await
            .text()
            .contains("unlock-bounced")
    );
    assert!(
        !server
            .get("/admin/unlock")
            .await
            .text()
            .contains("unlock-bounced"),
        "a reload must not repeat the notice"
    );
}

/// `/admin` keeps a one-line state note linking here, so the requirement is
/// discoverable before an action is refused rather than only after.
#[tokio::test]
async fn admin_links_to_the_page_while_locked() {
    let (server, _store, _dave) = locked_admin().await;
    let body = server.get("/admin").await.text();
    assert!(body.contains("elevation-confirm-link"), "{body}");
    assert!(body.contains("/admin/unlock"), "{body}");
}

/// A passwordless forward-auth admin has nothing to confirm, so the page says
/// so instead of showing a field they could never fill in.
#[tokio::test]
async fn the_page_tells_a_passwordless_admin_it_does_not_apply() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let uid = store
        .create_user("sso-admin", None, true, chrono::Utc::now())
        .await
        .unwrap();
    let session_id = pingward::auth::new_session_token();
    store
        .create_session(
            &session_id,
            uid,
            chrono::Utc::now() + chrono::Duration::hours(1),
            None,
            None,
            true,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    server.add_cookie(axum_extra::extract::cookie::Cookie::new(
        pingward::auth::session_cookie_name(false),
        pingward::secret::sign_session(common::TEST_SECRET.as_bytes(), &session_id),
    ));
    server.add_header(
        "x-csrf-token",
        pingward::secret::derive_csrf(common::TEST_SECRET.as_bytes(), &session_id),
    );

    let body = server.get("/admin/unlock").await.text();
    assert!(body.contains("unlock-not-applicable"), "{body}");
    assert!(!body.contains("unlock-input"), "{body}");
    // And /admin does not nag them about a gate that cannot apply.
    assert!(
        !server
            .get("/admin")
            .await
            .text()
            .contains("elevation-state")
    );

    // The gate really is inert for them, not merely hidden.
    let dave = store
        .create_user("dave", Some("x"), false, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post(&format!("/admin/users/{dave}/admin"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(dave).await.unwrap().unwrap().is_admin);
}

/// Regression lock for the message an admin sees after confirming.
///
/// It used to list the gated actions — "Confirmed. **Creating a user**,
/// resetting a password and granting admin are available…" — which, read by
/// someone who had just clicked "add user" and been bounced here, said their
/// user had been created. It had not: a refused action is dropped rather than
/// replayed, so the confirmation has to send them back to redo it.
#[tokio::test]
async fn confirming_does_not_claim_the_refused_action_succeeded() {
    let (server, store, _dave) = locked_admin().await;

    // Attempt something gated, get bounced, confirm.
    server
        .post("/admin/users")
        .form(&[("username", "carol"), ("password", "a long enough phrase")])
        .await;
    common::unlock_admin(&server, ADMIN_PW).await;

    let body = server.get("/admin").await.text();
    assert!(body.contains("elevation-flash"), "{body}");
    assert!(body.contains("was not performed"), "{body}");
    // Naming the actions is what made it readable as a success report.
    assert!(
        !body.contains("Creating a user, resetting"),
        "the confirmation must not list the gated actions: {body}"
    );
    // And the claim it must never make is the true one, checked against the
    // database rather than against the copy.
    assert!(
        store
            .find_user_by_username("carol")
            .await
            .unwrap()
            .is_none(),
        "nothing was created, so nothing may read as created"
    );
}

/// Validation runs before the gate, so a submission that could never succeed
/// says why instead of sending the admin through a confirmation for nothing.
///
/// This is the flow that produced the original report: submit a duplicate
/// username while locked, get bounced, confirm, and come back to a page that
/// looked like success. There is nothing to confirm now — the answer arrives
/// on the first submission.
#[tokio::test]
async fn a_doomed_submission_is_refused_without_asking_for_a_password() {
    let (server, store, _dave) = locked_admin().await;

    let res = server
        .post("/admin/users")
        // "admin" is this very session's own account.
        .form(&[("username", "admin"), ("password", "a long enough phrase")])
        .await;

    res.assert_status_ok(); // /admin re-rendered — no bounce
    assert!(res.text().contains("already exists"), "{}", res.text());
    // And no confirmation was demanded on the way.
    assert!(
        !server
            .get("/admin/unlock")
            .await
            .text()
            .contains("unlock-bounced")
    );
    assert_eq!(store.count_users().await.unwrap(), 2);
}

/// The reordering must not have moved the gate past a side effect: a *valid*
/// submission from a locked admin still writes nothing.
#[tokio::test]
async fn a_valid_submission_still_needs_confirming() {
    let (server, store, _dave) = locked_admin().await;
    server
        .post("/admin/users")
        .form(&[("username", "carol"), ("password", "a long enough phrase")])
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(
        store
            .find_user_by_username("carol")
            .await
            .unwrap()
            .is_none()
    );
}

// --- the in-page dialog's half of the contract ---
//
// `app.js` asks in place rather than letting the server bounce, so the form the
// admin filled in survives. That needs two things from the server: a reply it
// can act on without rendering HTML, and a marker telling it which controls
// will be refused. Both are checked here; the dialog itself is covered by the
// browser tests, which is the only place its behaviour is real.

/// `X-Requested-With: fetch` is this app's existing "answer me, do not navigate
/// me" signal. The decision is identical either way — only the presentation
/// differs — so a scripted caller is never a weaker door than the form.
#[tokio::test]
async fn the_fetch_variant_answers_with_status_codes() {
    let (server, store, dave) = locked_admin().await;

    let wrong = server
        .post("/admin/unlock")
        .add_header("x-requested-with", "fetch")
        .form(&[("password", "not-the-password")])
        .await;
    wrong.assert_status(StatusCode::FORBIDDEN);
    assert!(wrong.text().is_empty(), "no page to render into a dialog");

    let ok = server
        .post("/admin/unlock")
        .add_header("x-requested-with", "fetch")
        .form(&[("password", ADMIN_PW)])
        .await;
    ok.assert_status(StatusCode::NO_CONTENT);

    // And it really elevated, rather than merely answering politely.
    server
        .post(&format!("/admin/users/{dave}/admin"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(dave).await.unwrap().unwrap().is_admin);
}

#[tokio::test]
async fn the_fetch_variant_reports_the_lockout_too() {
    let (server, _store, _dave) = locked_admin().await;
    for _ in 0..pingward::ratelimit::ACCOUNT_MAX_ATTEMPTS {
        server
            .post("/admin/unlock")
            .add_header("x-requested-with", "fetch")
            .form(&[("password", "not-the-password")])
            .await;
    }
    let res = server
        .post("/admin/unlock")
        .add_header("x-requested-with", "fetch")
        .form(&[("password", ADMIN_PW)])
        .await;
    res.assert_status(StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        res.header("retry-after"),
        pingward::ratelimit::ACCOUNT_WINDOW_SECS.to_string()
    );
}

/// The marker follows the same granting-versus-removing rule the handlers do,
/// and disappears once confirmed. If it ever drifted from the handlers the
/// server would still refuse — the cost of drift is a needless dialog or a
/// bounce, never an ungated action.
#[tokio::test]
async fn only_the_granting_controls_are_marked_and_only_while_locked() {
    let (server, store, dave) = locked_admin().await;
    let body = server.get("/admin").await.text();
    assert!(body.contains(r#"data-reauth="create this user""#), "{body}");
    // One reset control per row, the signed-in admin's own included — that
    // form is deliberately not hidden behind `is_self`.
    assert_eq!(
        i64::try_from(body.matches(r#"data-reauth="reset this user"#).count()).unwrap(),
        store.count_users().await.unwrap(),
        "{body}"
    );
    assert!(
        body.contains(r#"data-reauth="grant admin rights""#),
        "{body}"
    );

    // Demoting goes through the same route and must not be marked — it is the
    // control an operator reaches for when they think they are under attack.
    store.set_user_admin(dave, true).await.unwrap();
    let body = server.get("/admin").await.text();
    assert!(
        !body.contains(r#"data-reauth="grant admin rights""#),
        "{body}"
    );

    // Confirmed: nothing is marked, so every form submits straight through.
    common::unlock_admin(&server, ADMIN_PW).await;
    assert!(!server.get("/admin").await.text().contains("data-reauth"));
}
