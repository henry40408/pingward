//! The elevation gate on `/admin` (`web::elevation`, `src/elevate.rs`): a
//! refused action bounces to the `/admin/unlock` interstitial. Granting access
//! (create user, reset password, promote) is gated; removing it (disable,
//! demote, delete) is not, so an operator under attack is never slowed down.

use axum::http::StatusCode;
use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};

mod common;

const ADMIN_PW: &str = "pw";

/// A signed-in admin, *not* unlocked, plus an ordinary account to act on.
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
    // The interstitial names what was refused.
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
        .post(&format!("/admin/users/{dave}/admin?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(!store.find_user_by_id(dave).await.unwrap().unwrap().is_admin);
}

// --- removing access is not gated ---

/// Demoting shares the promote handler but must stay ungated.
#[tokio::test]
async fn demoting_an_admin_works_while_locked() {
    let (server, store, dave) = locked_admin().await;
    store.set_user_admin(dave, true).await.unwrap();
    server
        .post(&format!("/admin/users/{dave}/admin?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(!store.find_user_by_id(dave).await.unwrap().unwrap().is_admin);
}

#[tokio::test]
async fn disabling_and_deleting_work_while_locked() {
    let (server, store, dave) = locked_admin().await;
    server
        .post(&format!("/admin/users/{dave}/disabled?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(dave).await.unwrap().unwrap().disabled);

    server
        .post(&format!("/admin/users/{dave}/delete?confirmed=1"))
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
    // The unlock page reports the live window rather than re-asking.
    let page = server.get("/admin/unlock").await.text();
    assert!(page.contains("unlock-state"), "{page}");
    assert!(!page.contains("unlock-input"), "{page}");

    server
        .post(&format!("/admin/users/{dave}/admin?confirmed=1"))
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
    // Re-renders with the error rather than redirecting.
    res.assert_status_ok();
    assert!(res.text().contains("That password is not correct."));
    assert!(res.text().contains("unlock-input"));

    server
        .post(&format!("/admin/users/{dave}/admin?confirmed=1"))
        .await;
    assert!(
        !store.find_user_by_id(dave).await.unwrap().unwrap().is_admin,
        "a failed unlock must not have elevated anything"
    );
}

/// Unlock shares the per-account limiter with `/login` and `/account`, so it
/// is not an unmetered password oracle.
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

/// Elevation is per session: another browser signed in as the same admin
/// stays locked.
#[tokio::test]
async fn elevation_does_not_leak_to_another_session() {
    let (server, store, dave) = locked_admin().await;
    common::unlock_admin(&server, ADMIN_PW).await;

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

    other
        .post(&format!("/admin/users/{dave}/admin?confirmed=1"))
        .await;
    assert!(
        !store.find_user_by_id(dave).await.unwrap().unwrap().is_admin,
        "the second session was never unlocked"
    );
}

/// Signing out and back in starts locked again. (A new session is locked
/// regardless; `Elevations::revoke` itself is unit-tested in `elevate.rs`.)
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

    again
        .post(&format!("/admin/users/{dave}/admin?confirmed=1"))
        .await;
    assert!(
        !store.find_user_by_id(dave).await.unwrap().unwrap().is_admin,
        "a fresh session after logout must start locked"
    );
}

// --- the interstitial page ---

/// The page explains why a signed-in admin is asked again, not just asks.
#[tokio::test]
async fn the_unlock_page_explains_the_requirement() {
    let (server, _store, _dave) = locked_admin().await;
    let body = server.get("/admin/unlock").await.text();

    assert!(body.contains("unlock-input"), "{body}");
    // Named with `<strong>`, not `.badge` (which reads as a status pill).
    assert!(body.contains("unlock-gated"), "{body}");
    assert!(
        body.contains("<strong>granting admin rights</strong>"),
        "{body}"
    );
    assert!(!body.contains("badge"), "{body}");
    assert!(body.contains("disabling, demoting, deleting"), "{body}");
    // Same password, not a second factor (no TOTP hunt).
    assert!(body.contains("not a second factor"), "{body}");
    // `ELEVATION_TTL_SECS`, rendered from the constant.
    assert!(body.contains("15m"), "{body}");
    assert!(body.contains("unlock-cancel"), "{body}");
}

/// Only a bounce shows the refused-action notice, and only once.
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

    server
        .post(&format!("/admin/users/{dave}/admin?confirmed=1"))
        .await;
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

/// `/admin` links to the page before any action is refused.
#[tokio::test]
async fn admin_links_to_the_page_while_locked() {
    let (server, _store, _dave) = locked_admin().await;
    let body = server.get("/admin").await.text();
    assert!(body.contains("elevation-confirm-link"), "{body}");
    assert!(body.contains("/admin/unlock"), "{body}");
}

/// A passwordless forward-auth admin has nothing to confirm: no field, and the
/// gate is inert.
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
    assert!(
        !server
            .get("/admin")
            .await
            .text()
            .contains("elevation-state")
    );

    // Inert, not merely hidden.
    let dave = store
        .create_user("dave", Some("x"), false, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post(&format!("/admin/users/{dave}/admin?confirmed=1"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(dave).await.unwrap().unwrap().is_admin);
}

/// A refused action is dropped, not replayed, so the post-unlock message must
/// say so rather than list the gated actions (which read as "user created").
#[tokio::test]
async fn confirming_does_not_claim_the_refused_action_succeeded() {
    let (server, store, _dave) = locked_admin().await;

    server
        .post("/admin/users")
        .form(&[("username", "carol"), ("password", "a long enough phrase")])
        .await;
    common::unlock_admin(&server, ADMIN_PW).await;

    let body = server.get("/admin").await.text();
    assert!(body.contains("elevation-flash"), "{body}");
    assert!(body.contains("was not performed"), "{body}");
    assert!(
        !body.contains("Creating a user, resetting"),
        "the confirmation must not list the gated actions: {body}"
    );
    assert!(
        store
            .find_user_by_username("carol")
            .await
            .unwrap()
            .is_none(),
        "nothing was created, so nothing may read as created"
    );
}

/// Validation runs before the gate: a submission that can never succeed says
/// why instead of demanding a confirmation first.
#[tokio::test]
async fn a_doomed_submission_is_refused_without_asking_for_a_password() {
    let (server, store, _dave) = locked_admin().await;

    let res = server
        .post("/admin/users")
        // Duplicate: "admin" already exists.
        .form(&[("username", "admin"), ("password", "a long enough phrase")])
        .await;

    res.assert_status_ok(); // /admin re-rendered — no bounce
    assert!(res.text().contains("already exists"), "{}", res.text());
    assert!(
        !server
            .get("/admin/unlock")
            .await
            .text()
            .contains("unlock-bounced")
    );
    assert_eq!(store.count_users().await.unwrap(), 2);
}

/// The gate still sits above the first side effect.
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

// --- the server half of `app.js`'s in-page unlock dialog ---
//
// It needs status-code replies and a `data-reauth` marker on gated controls;
// the dialog itself is covered by the browser tests.

/// With `X-Requested-With: fetch`, only the presentation changes (204/403),
/// not the decision.
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

    server
        .post(&format!("/admin/users/{dave}/admin?confirmed=1"))
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

/// `data-reauth` marks only granting controls, and only while locked.
#[tokio::test]
async fn only_the_granting_controls_are_marked_and_only_while_locked() {
    let (server, store, dave) = locked_admin().await;
    let body = server.get("/admin").await.text();
    assert!(body.contains(r#"data-reauth="create this user""#), "{body}");
    // One reset control per row, the admin's own included (not `is_self`-gated).
    assert_eq!(
        i64::try_from(body.matches(r#"data-reauth="reset this user"#).count()).unwrap(),
        store.count_users().await.unwrap(),
        "{body}"
    );
    assert!(
        body.contains(r#"data-reauth="grant admin rights""#),
        "{body}"
    );

    // Demoting shares the route and must not be marked.
    store.set_user_admin(dave, true).await.unwrap();
    let body = server.get("/admin").await.text();
    assert!(
        !body.contains(r#"data-reauth="grant admin rights""#),
        "{body}"
    );

    common::unlock_admin(&server, ADMIN_PW).await;
    assert!(!server.get("/admin").await.text().contains("data-reauth"));
}
