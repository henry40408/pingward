//! The elevation gate on `/admin`'s access-granting actions
//! (`web::elevation` / `src/elevate.rs`).
//!
//! `/admin`'s controls are single-button inline forms in a table row —
//! `users_toggle_admin` posts no body at all — so the per-action password field
//! `/account` uses does not fit. Re-authentication is decoupled instead: unlock
//! once via `POST /admin/unlock`, act for a while.
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
    assert!(
        server.get("/admin").await.text().contains("users-flash"),
        "the refusal has to say why, or nothing appears to have happened"
    );
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
    assert!(body.contains("Unlocked for another"), "{body}");

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
    res.assert_status_ok(); // re-rendered /admin with the error
    assert!(res.text().contains("That password is not correct."));

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
