use axum::http::StatusCode;
use axum_test::TestServer;
use pingward::{apikey, app, db, state::AppState, store::Store};

mod common;

/// A store shared by every `TestServer` built against it, plus one logged-in
/// **non-admin** member — account management is available to every
/// authenticated user, not just admins.
async fn member_store() -> (Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("member", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    (store, uid)
}

/// Log a fresh `TestServer` (its own cookie jar) into `store` as `username`,
/// with its own session's CSRF token attached as a default header so
/// protected POSTs pass `csrf_guard`. Ordered by `rowid` (not `created_at`)
/// so the just-inserted row is unambiguous even when a second session for the
/// same user already exists — `created_at`/`username` alone can't tell the
/// two apart, but `rowid` reflects strict insertion order.
async fn login_server(store: &Store, username: &str, password: &str) -> TestServer {
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    server
        .post("/login")
        .form(&[("username", username), ("password", password)])
        .await;
    let tok = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", tok.as_str());
    server
}

/// Pull the one-time plaintext token out of the create response (it's on the
/// copy button's `data-copy` attribute).
fn extract_token(html: &str) -> String {
    let marker = "data-copy=\"";
    let start = html.find(marker).expect("token banner present") + marker.len();
    let rest = &html[start..];
    let end = rest.find('"').unwrap();
    rest[..end].to_string()
}

#[tokio::test]
async fn nav_shows_account_link_for_member() {
    let (store, _uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    assert!(server.get("/").await.text().contains("nav-account"));
}

// --- sessions section ---

#[tokio::test]
async fn account_page_lists_and_marks_current_session() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;

    let res = server.get("/account").await;
    res.assert_status_ok();
    let body = res.text();

    // The session id IS the cookie's bearer secret: it must never be
    // rendered. Rows are identified by its SHA-256 handle instead.
    let sessions = store
        .list_sessions_for_user(uid, chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
    assert!(
        !body.contains(&sessions[0].id),
        "the raw session id (the cookie secret) must never be rendered"
    );
    assert!(body.contains(&format!(
        "session-row-{}",
        apikey::hash_api_key(&sessions[0].id)
    )));

    assert!(body.contains("session-current"), "current session marker");
    assert!(body.contains("session-row-"), "at least one session row");
    // Only the current session exists yet — no "revoke others" control.
    assert!(!body.contains("session-revoke-others"));
}

#[tokio::test]
async fn second_login_lists_two_sessions_and_revoke_others_leaves_one() {
    let (store, _uid) = member_store().await;
    let server1 = login_server(&store, "member", "pw").await;
    let _server2 = login_server(&store, "member", "pw").await;

    let body = server1.get("/account").await.text();
    assert_eq!(
        body.matches("session-row-").count(),
        2,
        "both sessions for the same user should be listed"
    );

    server1
        .post("/account/sessions/revoke-others")
        .await
        .assert_status(StatusCode::SEE_OTHER);

    let body = server1.get("/account").await.text();
    assert_eq!(
        body.matches("session-row-").count(),
        1,
        "only the current (server1) session should remain"
    );
    assert!(body.contains("session-current"));
}

#[tokio::test]
async fn revoking_the_current_session_logs_out() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;

    let sessions = store
        .list_sessions_for_user(uid, chrono::Utc::now())
        .await
        .unwrap();
    assert_eq!(sessions.len(), 1);
    let handle = apikey::hash_api_key(&sessions[0].id);

    server
        .post(&format!("/account/sessions/{handle}/revoke"))
        .await
        .assert_status(StatusCode::SEE_OTHER);

    // The session row and the cookie are both gone: the next request bounces
    // to /login instead of the dashboard.
    assert!(
        store
            .list_sessions_for_user(uid, chrono::Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
    server.get("/").await.assert_status(StatusCode::SEE_OTHER);
    let res = server.get("/account").await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/login");
}

#[tokio::test]
async fn unknown_or_foreign_handle_revokes_nothing() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;

    let other_uid = store
        .create_user("other", Some("x"), false, chrono::Utc::now())
        .await
        .unwrap();
    store
        .create_session(
            "other-session",
            other_uid,
            chrono::Utc::now() + chrono::Duration::hours(1),
            None,
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let other_handle = apikey::hash_api_key("other-session");

    // A garbage handle never 500s.
    server
        .post("/account/sessions/not-a-real-handle/revoke")
        .await
        .assert_status(StatusCode::SEE_OTHER);

    // Nor does another user's real handle — that session survives.
    server
        .post(&format!("/account/sessions/{other_handle}/revoke"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert_eq!(
        store
            .list_sessions_for_user(other_uid, chrono::Utc::now())
            .await
            .unwrap()
            .len(),
        1
    );
    // The caller's own session (used above to authenticate) is unaffected.
    assert_eq!(
        store
            .list_sessions_for_user(uid, chrono::Utc::now())
            .await
            .unwrap()
            .len(),
        1
    );
}

// --- API keys section ---

#[tokio::test]
async fn create_shows_token_once_then_only_the_prefix() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    server.get("/account").await.assert_status_ok();

    let res = server
        .post("/account/api-keys")
        .form(&[("name", "CI deploy"), ("expires_in", "")])
        .await;
    res.assert_status_ok();
    let token = extract_token(&res.text());
    assert!(token.starts_with("pw_"));
    assert_eq!(token.len(), 67); // "pw_" + 64 hex

    // Persisted for this user, and the hash resolves back to the user.
    let keys = store.list_api_keys_for_user(uid).await.unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(
        store
            .validate_api_key(&apikey::hash_api_key(&token), chrono::Utc::now())
            .await
            .unwrap(),
        Some(uid)
    );

    // Reloading the list never re-exposes the plaintext — only the prefix.
    let body = server.get("/account").await.text();
    assert!(!body.contains(&token), "plaintext token must not reappear");
    assert!(body.contains(&keys[0].prefix));
}

#[tokio::test]
async fn account_page_links_to_the_docs() {
    let (store, _uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    let body = server.get("/account").await.text();
    // The page points users at the interactive reference and the raw spec.
    assert!(body.contains("data-testid=\"api-docs-link\""));
    assert!(body.contains("href=\"/api/docs\""));
    assert!(body.contains("href=\"/api/openapi.json\""));
}

#[tokio::test]
async fn expired_key_is_flagged_but_a_live_one_is_not() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    let now = chrono::Utc::now();

    // A key whose expiry is already in the past.
    let (_f1, p1, h1) = apikey::generate_api_key();
    let dead = store
        .insert_api_key(
            uid,
            "dead",
            &h1,
            &p1,
            Some(now - chrono::Duration::hours(1)),
            now,
        )
        .await
        .unwrap();
    // A key that is still valid (or never expires).
    let (_f2, p2, h2) = apikey::generate_api_key();
    let live = store
        .insert_api_key(
            uid,
            "live",
            &h2,
            &p2,
            Some(now + chrono::Duration::days(30)),
            now,
        )
        .await
        .unwrap();

    let body = server.get("/account").await.text();
    assert!(
        body.contains(&format!("api-key-expired-{dead}")),
        "expired key should carry the expired badge"
    );
    assert!(
        !body.contains(&format!("api-key-expired-{live}")),
        "a live key must not be flagged expired"
    );
}

#[tokio::test]
async fn keys_are_caller_scoped() {
    let (store, _uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    let now = chrono::Utc::now();
    let other = store
        .create_user("other", Some("x"), false, now)
        .await
        .unwrap();
    let (_full, prefix, hash) = apikey::generate_api_key();
    let other_kid = store
        .insert_api_key(other, "theirs", &hash, &prefix, None, now)
        .await
        .unwrap();

    // The member's list shows nothing belonging to `other`.
    let body = server.get("/account").await.text();
    assert!(!body.contains("theirs"));
    assert!(!body.contains(&prefix));

    // And they can't revoke it — the delete is a silent no-op, key survives.
    server
        .post(&format!("/account/api-keys/{other_kid}/delete"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert_eq!(store.list_api_keys_for_user(other).await.unwrap().len(), 1);
}

#[tokio::test]
async fn revoke_own_key() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    let now = chrono::Utc::now();
    let (_full, prefix, hash) = apikey::generate_api_key();
    let kid = store
        .insert_api_key(uid, "k", &hash, &prefix, None, now)
        .await
        .unwrap();

    server
        .post(&format!("/account/api-keys/{kid}/delete"))
        .await
        .assert_status(StatusCode::SEE_OTHER);
    assert!(store.list_api_keys_for_user(uid).await.unwrap().is_empty());
}

#[tokio::test]
async fn create_without_csrf_is_forbidden() {
    // Log in but never install the CSRF header, proving the route sits inside
    // csrf_guard (unlike the machine ping API).
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("member", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", "member"), ("password", "pw")])
        .await;

    let res = server
        .post("/account/api-keys")
        .form(&[("name", "x"), ("expires_in", "")])
        .await;
    res.assert_status(StatusCode::FORBIDDEN);
    assert!(store.list_api_keys_for_user(uid).await.unwrap().is_empty());
}

#[tokio::test]
async fn create_with_expiry_sets_expires_at() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    server
        .post("/account/api-keys")
        .form(&[("name", "temp"), ("expires_in", "30d")])
        .await
        .assert_status_ok();
    let keys = store.list_api_keys_for_user(uid).await.unwrap();
    assert!(keys[0].expires_at.is_some());
}

#[tokio::test]
async fn create_with_bad_expiry_is_rejected() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    let res = server
        .post("/account/api-keys")
        .form(&[("name", "temp"), ("expires_in", "banana")])
        .await;
    res.assert_status_ok();
    assert!(res.text().contains("expiry must be"));
    assert!(store.list_api_keys_for_user(uid).await.unwrap().is_empty());
}

#[tokio::test]
async fn create_with_blank_name_is_rejected() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    let res = server
        .post("/account/api-keys")
        .form(&[("name", "   "), ("expires_in", "")])
        .await;
    res.assert_status_ok();
    assert!(res.text().contains("a name is required"));
    assert!(store.list_api_keys_for_user(uid).await.unwrap().is_empty());
}

#[tokio::test]
async fn validate_rejects_expired_and_unknown_keys() {
    let (store, uid) = member_store().await;
    let now = chrono::Utc::now();
    let (_full, prefix, hash) = apikey::generate_api_key();
    store
        .insert_api_key(
            uid,
            "old",
            &hash,
            &prefix,
            Some(now - chrono::Duration::hours(1)),
            now,
        )
        .await
        .unwrap();
    // Expired → rejected.
    assert_eq!(store.validate_api_key(&hash, now).await.unwrap(), None);
    // Unknown hash → rejected.
    assert_eq!(store.validate_api_key("deadbeef", now).await.unwrap(), None);
}
