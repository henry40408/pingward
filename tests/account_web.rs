use axum::http::StatusCode;
use axum_test::TestServer;
use pingward::{apikey, app, db, state::AppState, store::Store};

mod common;

/// A store shared by every `TestServer` built against it, plus one *non-admin*
/// member: account management is available to every authenticated user, not
/// just admins.
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
/// with that session's CSRF token attached as a default header so protected
/// POSTs pass `csrf_guard`. The session is found by `rowid`: a second session
/// for the same user is indistinguishable by `created_at`/`username`.
async fn login_server(store: &Store, username: &str, password: &str) -> TestServer {
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", username),
            ("password", password),
        ])
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
async fn password_login_session_has_no_sso_pill() {
    let (store, _uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;

    let body = server.get("/account").await.text();
    // Guard against a vacuous pass: a session row must have rendered before
    // asserting the SSO pill is absent from it.
    assert!(
        body.contains("session-current"),
        "account page rendered a session"
    );
    assert!(
        !body.contains("session-sso"),
        "a plain password-login session must not be flagged SSO"
    );
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
        .post("/account/sessions/revoke-others?confirmed=1")
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
        .post(&format!("/account/sessions/{handle}/revoke?confirmed=1"))
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
            false,
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
        .post(&format!(
            "/account/sessions/{other_handle}/revoke?confirmed=1"
        ))
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

// --- password section ---

/// The replacement password these tests set. It has to clear
/// `auth::validate_password`'s length floor, which applies to every surface that
/// *sets* a password — never to `/login`, so the fixtures' short `"pw"` still
/// signs in.
const NEW_PW: &str = "a whole new passphrase";

/// The stored PHC hash, read straight out of the table so a test can assert on
/// the credential itself rather than on a login round-trip.
async fn stored_hash(store: &Store, uid: i64) -> String {
    store
        .find_user_by_id(uid)
        .await
        .unwrap()
        .unwrap()
        .password_hash
        .expect("password account")
}

#[tokio::test]
async fn changing_the_password_rotates_it_and_signs_out_other_sessions() {
    let (store, uid) = member_store().await;
    let server1 = login_server(&store, "member", "pw").await;
    let server2 = login_server(&store, "member", "pw").await;
    assert_eq!(session_count(&store, uid).await, 2);

    server1
        .post("/account/password")
        .form(&[
            ("current_password", "pw"),
            ("new_password", NEW_PW),
            ("confirm_password", NEW_PW),
        ])
        .await
        .assert_status(StatusCode::SEE_OTHER);

    let phc = stored_hash(&store, uid).await;
    assert!(pingward::auth::verify_password(NEW_PW, &phc));
    assert!(!pingward::auth::verify_password("pw", &phc));

    // The session that made the change survives; the other is gone, so changing
    // a password to evict someone actually evicts them.
    assert_eq!(session_count(&store, uid).await, 1);
    let body = server1.get("/account").await.text();
    assert!(body.contains("password-changed-flash"), "{body}");
    // One-shot: a reload does not repeat the notice.
    assert!(
        !server1
            .get("/account")
            .await
            .text()
            .contains("password-changed-flash")
    );
    let res = server2.get("/account").await;
    res.assert_status(StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/login");
}

#[tokio::test]
async fn changing_the_password_leaves_api_keys_alone() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    server
        .post("/account/api-keys")
        .form(&[("name", "ci"), ("current_password", "pw")])
        .await
        .assert_status_ok();

    server
        .post("/account/password")
        .form(&[
            ("current_password", "pw"),
            ("new_password", NEW_PW),
            ("confirm_password", NEW_PW),
        ])
        .await
        .assert_status(StatusCode::SEE_OTHER);

    assert_eq!(
        store.list_api_keys_for_user(uid).await.unwrap().len(),
        1,
        "a password change revokes sessions, not keys — /account says so"
    );
}

/// Each rejection path: the credential is untouched and no session is revoked,
/// so a wrong guess is never a way to sign someone else's browser out.
#[tokio::test]
async fn rejected_changes_touch_neither_the_password_nor_the_sessions() {
    for (label, current, new, confirm) in [
        ("wrong current password", "nope", NEW_PW, NEW_PW),
        ("mismatched confirmation", "pw", NEW_PW, "different"),
        ("blank new password", "pw", "", ""),
        // Below `auth::MIN_PASSWORD_CHARS`, rejected on the same path as a
        // blank one.
        (
            "new password under the floor",
            "pw",
            "short pass",
            "short pass",
        ),
    ] {
        let (store, uid) = member_store().await;
        let server1 = login_server(&store, "member", "pw").await;
        let _server2 = login_server(&store, "member", "pw").await;
        let before = stored_hash(&store, uid).await;

        let res = server1
            .post("/account/password")
            .form(&[
                ("current_password", current),
                ("new_password", new),
                ("confirm_password", confirm),
            ])
            .await;
        res.assert_status_ok();
        assert!(res.text().contains("password-error"), "{label}");

        assert_eq!(stored_hash(&store, uid).await, before, "{label}");
        assert_eq!(session_count(&store, uid).await, 2, "{label}");
    }
}

/// A forward-auth account has no local password to verify against, so it gets no
/// form, and posting anyway is refused rather than setting a first one — that
/// would be a second way in the gateway's sign-out cannot end.
#[tokio::test]
async fn a_passwordless_account_has_no_form_and_cannot_set_one() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let uid = store
        .create_user("sso-user", None, false, chrono::Utc::now())
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

    let body = server.get("/account").await.text();
    assert!(!body.contains("password-submit"), "{body}");

    server
        .post("/account/password")
        .form(&[
            ("current_password", ""),
            ("new_password", NEW_PW),
            ("confirm_password", NEW_PW),
        ])
        .await
        .assert_status(StatusCode::FORBIDDEN);
    assert!(
        store
            .find_user_by_id(uid)
            .await
            .unwrap()
            .unwrap()
            .password_hash
            .is_none()
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
        .form(&[
            ("name", "CI deploy"),
            ("expires_in", ""),
            ("current_password", "pw"),
        ])
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
    assert!(body.contains("data-testid=\"api-docs-link\""));
    assert!(body.contains("href=\"/api/docs\""));
    assert!(body.contains("href=\"/api/openapi.json\""));
}

#[tokio::test]
async fn expired_key_is_flagged_but_a_live_one_is_not() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    let now = chrono::Utc::now();

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
        .post(&format!("/account/api-keys/{other_kid}/delete?confirmed=1"))
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
        .post(&format!("/account/api-keys/{kid}/delete?confirmed=1"))
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
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "member"),
            ("password", "pw"),
        ])
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
        .form(&[
            ("name", "temp"),
            ("expires_in", "30d"),
            ("current_password", "pw"),
        ])
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
        .form(&[
            ("name", "temp"),
            ("expires_in", "banana"),
            ("current_password", "pw"),
        ])
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
        .form(&[
            ("name", "   "),
            ("expires_in", ""),
            ("current_password", "pw"),
        ])
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
    assert_eq!(store.validate_api_key(&hash, now).await.unwrap(), None);
    assert_eq!(store.validate_api_key("deadbeef", now).await.unwrap(), None);
}

/// A session past the absolute cap (`created_at + 30d`) is already inert —
/// `find_session_user` refuses it — but it used to stay in the table, hidden
/// from `/account` until the next prune pass, so its owner could neither see it
/// nor revoke it. Opening the page reaps it, so "not listed" means "gone".
#[tokio::test]
async fn opening_the_account_page_reaps_a_session_past_the_absolute_cap() {
    let (store, uid) = member_store().await;
    let now = chrono::Utc::now();
    // Older than the cap yet carrying a still-future `expires_at`: the shape a
    // build that *lowers* the cap leaves behind, and the only one neither
    // refused by the `expires_at` predicate nor reaped by prune's.
    store
        .create_session(
            "capped-session",
            uid,
            now + chrono::Duration::hours(1),
            Some("curl/8"),
            Some("203.0.113.5"),
            false,
            now - chrono::Duration::days(31),
        )
        .await
        .unwrap();
    assert_eq!(session_count(&store, uid).await, 1, "seeded");

    let server = login_server(&store, "member", "pw").await;
    assert_eq!(
        session_count(&store, uid).await,
        2,
        "the login added a second, live session"
    );

    let body = server.get("/account").await.text();
    assert!(
        !body.contains("203.0.113.5"),
        "a capped session must not be listed: {body}"
    );
    assert_eq!(
        session_count(&store, uid).await,
        1,
        "the capped row must be deleted, not merely hidden — the live session stays"
    );
}

/// The reap is scoped to the caller: one user opening `/account` must not
/// touch another user's rows.
#[tokio::test]
async fn the_reap_does_not_touch_another_users_sessions() {
    let (store, uid) = member_store().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let other = store
        .create_user("other", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    let now = chrono::Utc::now();
    for (id, owner) in [("mine", uid), ("theirs", other)] {
        store
            .create_session(
                id,
                owner,
                now + chrono::Duration::hours(1),
                None,
                None,
                false,
                now - chrono::Duration::days(31),
            )
            .await
            .unwrap();
    }

    let server = login_server(&store, "member", "pw").await;
    server.get("/account").await.assert_status_ok();

    // Two-sided: the caller's own capped row must be gone, or "the other user's
    // survived" would hold vacuously with no reap running at all.
    assert_eq!(
        session_count(&store, uid).await,
        1,
        "the caller's own capped session must be reaped"
    );
    assert_eq!(
        session_count(&store, other).await,
        1,
        "the other user's capped session must survive"
    );
}

/// Sessions belonging to `user_id`, counted straight out of the table so the
/// assertion sees rows the handlers deliberately hide.
async fn session_count(store: &Store, user_id: i64) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(&store.pool)
        .await
        .unwrap()
}

// --- re-authentication before minting an API key ---
//
// An API key outlives the session that minted it: it is bound by neither the
// idle nor the absolute session cap, and `users_set_password` leaves it alone.
// A borrowed browser would otherwise turn one session's access into permanent
// access — the one gated action signing out cannot undo.

#[tokio::test]
async fn creating_a_key_without_the_password_is_refused() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;

    let res = server
        .post("/account/api-keys")
        .form(&[("name", "ci"), ("expires_in", ""), ("current_password", "")])
        .await;

    res.assert_status_ok();
    assert!(res.text().contains("api-key-error"), "{}", res.text());
    assert!(
        store.list_api_keys_for_user(uid).await.unwrap().is_empty(),
        "a refused re-authentication must not have minted a key"
    );
}

#[tokio::test]
async fn creating_a_key_with_the_wrong_password_is_refused() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;

    let res = server
        .post("/account/api-keys")
        .form(&[
            ("name", "ci"),
            ("expires_in", ""),
            ("current_password", "not-my-password"),
        ])
        .await;

    res.assert_status_ok();
    assert!(res.text().contains("Current password is incorrect."));
    assert!(store.list_api_keys_for_user(uid).await.unwrap().is_empty());
}

/// The gate is checked *before* the name and expiry, so the validation errors
/// cannot serve as an oracle and a wrong password never reaches the rest of the
/// handler.
#[tokio::test]
async fn the_password_is_checked_before_the_rest_of_the_form() {
    let (store, _uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;

    let res = server
        .post("/account/api-keys")
        .form(&[
            ("name", "   "),          // also invalid
            ("expires_in", "banana"), // also invalid
            ("current_password", "not-my-password"),
        ])
        .await;

    let body = res.text();
    assert!(body.contains("Current password is incorrect."), "{body}");
    assert!(!body.contains("a name is required"), "{body}");
    assert!(!body.contains("expiry must be a duration"), "{body}");
}

/// The form renders the field, so the requirement is discoverable rather than
/// a submission-time surprise.
#[tokio::test]
async fn the_key_form_asks_for_the_password() {
    let (store, _uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;
    let body = server.get("/account").await.text();
    assert!(body.contains("api-key-password-input"), "{body}");
}

/// Guessing the owner's password from a stolen session lands in the same account
/// bucket as guessing it at the login form. The form was previously an unmetered
/// password oracle.
#[tokio::test]
async fn repeated_wrong_passwords_exhaust_the_account_budget() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;

    for _ in 0..pingward::ratelimit::ACCOUNT_MAX_ATTEMPTS {
        server
            .post("/account/api-keys")
            .form(&[
                ("name", "ci"),
                ("expires_in", ""),
                ("current_password", "not-my-password"),
            ])
            .await
            .assert_status_ok();
    }

    // Budget spent: even the *correct* password is now refused, and the
    // message says so rather than claiming the password was wrong.
    let res = server
        .post("/account/api-keys")
        .form(&[
            ("name", "ci"),
            ("expires_in", ""),
            ("current_password", "pw"),
        ])
        .await;
    assert!(res.text().contains("Too many attempts"), "{}", res.text());
    assert!(store.list_api_keys_for_user(uid).await.unwrap().is_empty());
}

/// A success clears the bucket, so an owner who fumbles their way to the edge
/// and then gets it right is not left one mistake from a lockout.
#[tokio::test]
async fn a_correct_password_clears_the_account_budget() {
    let (store, uid) = member_store().await;
    let server = login_server(&store, "member", "pw").await;

    for _ in 0..pingward::ratelimit::ACCOUNT_MAX_ATTEMPTS - 1 {
        server
            .post("/account/api-keys")
            .form(&[
                ("name", "ci"),
                ("expires_in", ""),
                ("current_password", "not-my-password"),
            ])
            .await;
    }
    server
        .post("/account/api-keys")
        .form(&[
            ("name", "ci"),
            ("expires_in", ""),
            ("current_password", "pw"),
        ])
        .await
        .assert_status_ok();
    assert_eq!(store.list_api_keys_for_user(uid).await.unwrap().len(), 1);

    // A full budget again: a refund of one would have run out immediately.
    for _ in 0..pingward::ratelimit::ACCOUNT_MAX_ATTEMPTS {
        let res = server
            .post("/account/api-keys")
            .form(&[
                ("name", "ci2"),
                ("expires_in", ""),
                ("current_password", "not-my-password"),
            ])
            .await;
        assert!(!res.text().contains("Too many attempts"));
    }
}

/// A passwordless forward-auth account passes the gate unchallenged, and is not
/// shown a field it could never fill in.
///
/// There is no stored credential to verify and no protocol for asking the
/// gateway to re-assert its authority, so a *borrowed* forward-auth session can
/// still mint a key; refusing would leave those users with no way to get one.
/// The opposite outcome to `/account/password`, which 403s such an account —
/// that one would be *setting* a local password, a second way in the gateway's
/// sign-out cannot end.
#[tokio::test]
async fn a_passwordless_account_mints_a_key_without_re_authenticating() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let uid = store
        .create_user("sso-user", None, false, chrono::Utc::now())
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

    let body = server.get("/account").await.text();
    assert!(
        !body.contains("api-key-password-input"),
        "no stored password means no field to render: {body}"
    );

    // The posted form genuinely has no `current_password` key — which is why
    // `NewApiKeyForm` defaults it rather than requiring it.
    server
        .post("/account/api-keys")
        .form(&[("name", "ci"), ("expires_in", "")])
        .await
        .assert_status_ok();
    assert_eq!(store.list_api_keys_for_user(uid).await.unwrap().len(), 1);
}
