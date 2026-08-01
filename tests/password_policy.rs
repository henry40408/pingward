//! The password length policy (`auth::validate_password`) across every surface
//! that *sets* a password.
//!
//! There are four: `/setup`, `/admin/users`, `/admin/users/{id}/password` and
//! `/account/password`. The last is covered in `tests/account_web.rs` alongside
//! the rest of that page's rejection paths; the other three are here, together,
//! because the failure mode this guards against is a *new* surface (or a
//! restored one) quietly not calling the validator — which no per-page test
//! would notice.
//!
//! `/login` is deliberately absent, and must stay absent: validating on
//! sign-in would lock out every account whose password predates the policy,
//! and the length of a submitted password is not evidence of anything.

use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};

mod common;

/// Below `auth::MIN_PASSWORD_CHARS`.
const TOO_SHORT: &str = "short pass";
/// At or above it.
const LONG_ENOUGH: &str = "a perfectly fine passphrase";

async fn server() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    (server, store)
}

/// A server signed in as an admin, with the session's CSRF token set as a
/// default header so protected POSTs are not rejected by `csrf_guard`.
async fn admin_server() -> (TestServer, Store, i64) {
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
    let tok = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", tok.as_str());
    common::unlock_admin(&server, "pw").await;
    (server, store, uid)
}

#[tokio::test]
async fn setup_refuses_a_password_under_the_floor() {
    let (mut server, store) = server().await;
    let csrf = common::anonymous_csrf(&mut server).await;
    let res = server
        .post("/setup")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "admin"),
            ("password", TOO_SHORT),
        ])
        .await;

    res.assert_status_ok(); // the form, re-rendered
    let body = res.text();
    assert!(body.contains("setup-error"), "{body}");
    assert!(
        body.contains(&format!(
            "at least {} characters",
            pingward::auth::MIN_PASSWORD_CHARS
        )),
        "the message must say what the requirement is: {body}"
    );
    assert_eq!(
        store.count_users().await.unwrap(),
        0,
        "a rejected /setup must not have created the admin"
    );
}

/// The pair-wise message survives for a genuinely blank submission — the
/// password policy only speaks once there is a username to go with it.
#[tokio::test]
async fn setup_still_reports_a_missing_username_as_a_pair() {
    let (mut server, store) = server().await;
    let csrf = common::anonymous_csrf(&mut server).await;
    let res = server
        .post("/setup")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", ""),
            ("password", LONG_ENOUGH),
        ])
        .await;

    res.assert_status_ok();
    assert!(res.text().contains("username and password are required"));
    assert_eq!(store.count_users().await.unwrap(), 0);
}

#[tokio::test]
async fn setup_accepts_a_password_at_the_floor() {
    let (mut server, store) = server().await;
    let csrf = common::anonymous_csrf(&mut server).await;
    let at_the_floor = "a".repeat(pingward::auth::MIN_PASSWORD_CHARS);
    server
        .post("/setup")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "admin"),
            ("password", at_the_floor.as_str()),
        ])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(store.count_users().await.unwrap(), 1);
}

#[tokio::test]
async fn admin_user_creation_refuses_a_password_under_the_floor() {
    let (server, store, _admin) = admin_server().await;
    let res = server
        .post("/admin/users")
        .form(&[("username", "carol"), ("password", TOO_SHORT)])
        .await;

    res.assert_status_ok(); // /admin, re-rendered with the error
    assert!(res.text().contains(&format!(
        "at least {} characters",
        pingward::auth::MIN_PASSWORD_CHARS
    )));
    assert!(
        store
            .find_user_by_username("carol")
            .await
            .unwrap()
            .is_none(),
        "a rejected creation must not have created the account"
    );
}

/// The reset path used to answer a bad password with a bare redirect back to
/// `/admin`, which is indistinguishable from success — an admin would believe
/// they had rotated a credential they had not. It now renders the reason.
#[tokio::test]
async fn admin_password_reset_refuses_and_says_so_rather_than_redirecting() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let dave = store
        .create_user("dave", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();

    let res = server
        .post(&format!("/admin/users/{dave}/password"))
        .form(&[("password", TOO_SHORT)])
        .await;

    res.assert_status_ok();
    assert!(res.text().contains(&format!(
        "at least {} characters",
        pingward::auth::MIN_PASSWORD_CHARS
    )));

    let stored = store
        .find_user_by_id(dave)
        .await
        .unwrap()
        .unwrap()
        .password_hash
        .unwrap();
    assert_eq!(stored, phc, "the stored credential must be untouched");
    assert!(
        !store
            .list_audit(50)
            .await
            .unwrap()
            .iter()
            .any(|a| a.action == "user.password_reset"),
        "a refused reset is not a reset, and must not be audited as one"
    );
}

/// Over `auth::MAX_PASSWORD_CHARS` is a rejection, never a silent truncation —
/// a truncated password would authenticate a shorter prefix than the user
/// believes they set.
#[tokio::test]
async fn an_over_long_password_is_refused_not_truncated() {
    let (server, store, _admin) = admin_server().await;
    let huge = "a".repeat(pingward::auth::MAX_PASSWORD_CHARS + 1);
    let res = server
        .post("/admin/users")
        .form(&[("username", "carol"), ("password", huge.as_str())])
        .await;

    res.assert_status_ok();
    assert!(res.text().contains(&format!(
        "at most {} characters",
        pingward::auth::MAX_PASSWORD_CHARS
    )));
    assert!(
        store
            .find_user_by_username("carol")
            .await
            .unwrap()
            .is_none()
    );
}
