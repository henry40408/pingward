use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};

mod common;

/// After a session exists, send its CSRF token as a default `X-CSRF-Token`
/// header so protected POSTs pass `csrf_guard`. Call after every (re)login.
async fn set_csrf(server: &mut TestServer, store: &Store) {
    let tok = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", tok.as_str());
}

/// Log a fresh `TestServer` (its own cookie jar) into `store` as `username`,
/// mirroring `account_web.rs`'s `login_server` — used so the target user's
/// session lives on a separate `TestServer`/cookie jar from the admin's,
/// letting a test check both after a privilege-level change on the target.
async fn login_as(store: &Store, username: &str, password: &str) -> TestServer {
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
    server
}

async fn admin_server() -> (TestServer, Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    let admin_id = store
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
    set_csrf(&mut server, &store).await;
    common::unlock_admin(&server, "pw").await;
    (server, store, admin_id)
}

#[tokio::test]
async fn creating_user_is_audited() {
    let (server, store, _admin) = admin_server().await;
    server
        .post("/admin/users")
        .form(&[("username", "carol"), ("password", "carol's passphrase")])
        .await;
    let carol = store.find_user_by_username("carol").await.unwrap().unwrap();
    let audit = store.list_audit(50).await.unwrap();
    assert!(audit.iter().any(|a| a.action == "user.create"
        && a.target_type.as_deref() == Some("user")
        && a.target_id == Some(carol.id)));
}

#[tokio::test]
async fn deleting_user_is_audited() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let dave = store
        .create_user("dave", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post(&format!("/admin/users/{dave}/delete?confirmed=1"))
        .await;
    let audit = store.list_audit(50).await.unwrap();
    assert!(audit.iter().any(|a| a.action == "user.delete"
        && a.target_type.as_deref() == Some("user")
        && a.target_id == Some(dave)));
}

#[tokio::test]
async fn deleting_nonexistent_user_writes_no_audit() {
    let (server, store, _admin) = admin_server().await;
    let before = store.list_audit(50).await.unwrap().len();
    server.post("/admin/users/99999/delete?confirmed=1").await; // nonexistent id
    let after = store.list_audit(50).await.unwrap();
    assert!(
        !after
            .iter()
            .any(|a| a.action == "user.delete" && a.target_id == Some(99999))
    );
    assert_eq!(after.len(), before);
}

#[tokio::test]
async fn resetting_password_for_nonexistent_user_writes_no_audit() {
    let (server, store, _admin) = admin_server().await;
    server
        .post("/admin/users/99999/password")
        .form(&[("password", "whatever passphrase")])
        .await;
    assert!(
        !store
            .list_audit(50)
            .await
            .unwrap()
            .iter()
            .any(|a| a.action == "user.password_reset" && a.target_id == Some(99999))
    );
}

#[tokio::test]
async fn promote_and_demote_admin() {
    let (server, store, _admin) = admin_server().await;
    let uid = store
        .create_user("erin", Some("p"), false, chrono::Utc::now())
        .await
        .unwrap();
    // promote
    server
        .post(&format!("/admin/users/{uid}/admin?confirmed=1"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(uid).await.unwrap().unwrap().is_admin);
    // demote back
    server
        .post(&format!("/admin/users/{uid}/admin?confirmed=1"))
        .await;
    assert!(!store.find_user_by_id(uid).await.unwrap().unwrap().is_admin);
    assert!(
        store
            .list_audit(50)
            .await
            .unwrap()
            .iter()
            .any(|a| a.action == "user.set_admin")
    );
}

#[tokio::test]
async fn cannot_demote_self() {
    let (server, store, admin_id) = admin_server().await;
    // Only one admin exists here, so this alone can't distinguish the
    // self-guard from the (provably unreachable) last-admin guard; see
    // `demoting_self_is_refused_with_flash_even_with_a_second_admin` below
    // for a case that isolates the self-guard.
    server
        .post(&format!("/admin/users/{admin_id}/admin?confirmed=1"))
        .await;
    assert!(
        store
            .find_user_by_id(admin_id)
            .await
            .unwrap()
            .unwrap()
            .is_admin
    );
}

#[tokio::test]
async fn demoting_self_is_refused_with_flash_even_with_a_second_admin() {
    let (server, store, admin_id) = admin_server().await;
    // A second enabled admin exists, so if the self-guard were absent,
    // count_enabled_admins() would be >= 2 and the last-admin guard could
    // not explain a refusal either — isolating the self-guard as the cause.
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("gale", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();

    let res = server
        .post(&format!("/admin/users/{admin_id}/admin?confirmed=1"))
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(
        store
            .find_user_by_id(admin_id)
            .await
            .unwrap()
            .unwrap()
            .is_admin,
        "self-demote must be refused even though a second enabled admin exists"
    );
    let flash = res.maybe_cookie("pingward_flash");
    assert_eq!(
        flash
            .as_ref()
            .and_then(|c| common::flash_payload(c.value())),
        Some("users_blocked".to_string()),
        "the flash must carry a signed users_blocked payload: {flash:?}"
    );

    let body = server.get("/admin").await.text();
    assert!(
        body.contains("data-testid=\"users-flash\""),
        "flash should render once: {body}"
    );

    let body2 = server.get("/admin").await.text();
    assert!(
        !body2.contains("data-testid=\"users-flash\""),
        "flash must be one-shot: {body2}"
    );
}

#[tokio::test]
async fn admin_resets_password_and_target_can_login() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("original").unwrap();
    store
        .create_user("dave", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    let dave = store.find_user_by_username("dave").await.unwrap().unwrap();

    // Dave establishes a session before the reset — this is the session an
    // intruder using his old password would be sitting on.
    let dave_server = login_as(&store, "dave", "original").await;
    dave_server.get("/account").await.assert_status_ok();

    server
        .post(&format!("/admin/users/{}/password", dave.id))
        .form(&[("password", "a brand new passphrase")])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    let updated = store.find_user_by_id(dave.id).await.unwrap().unwrap();
    assert!(pingward::auth::verify_password(
        "a brand new passphrase",
        updated.password_hash.as_deref().unwrap()
    ));
    assert!(
        store
            .list_audit(50)
            .await
            .unwrap()
            .iter()
            .any(|a| a.action == "user.password_reset" && a.target_id == Some(dave.id))
    );

    // OWASP: the password reset must invalidate Dave's existing session, not
    // just reject future logins with the old password.
    assert!(
        store
            .list_sessions_for_user(dave.id, chrono::Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
    let res = dave_server.get("/account").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/login");
}

/// Regression: `templates/admin.html` renders the password-reset form for
/// every row, including the admin's own — unlike delete/toggle-admin/
/// toggle-disabled it is not hidden behind `is_self`. Resetting your own
/// password must not sign out the browser you are using to do it, only every
/// *other* session belonging to the same account.
#[tokio::test]
async fn admin_resets_own_password_keeps_current_session() {
    let (server, store, admin_id) = admin_server().await;
    // A second session for the same admin — e.g. another browser/device —
    // must still be revoked by the reset.
    let other_admin_session = login_as(&store, "admin", "pw").await;
    other_admin_session.get("/account").await.assert_status_ok();
    assert_eq!(
        store
            .list_sessions_for_user(admin_id, chrono::Utc::now())
            .await
            .unwrap()
            .len(),
        2
    );

    server
        .post(&format!("/admin/users/{admin_id}/password"))
        .form(&[("password", "a brand new passphrase")])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);

    // (a) the admin's own session, the one that issued the reset, still works.
    server.get("/account").await.assert_status_ok();

    // (b) the other session belonging to the same admin is gone.
    let res = other_admin_session.get("/account").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/login");

    assert_eq!(
        store
            .list_sessions_for_user(admin_id, chrono::Utc::now())
            .await
            .unwrap()
            .len(),
        1
    );

    // (c) the new password now logs in.
    let relogged = login_as(&store, "admin", "a brand new passphrase").await;
    relogged.get("/account").await.assert_status_ok();
}

/// Password reset revokes sessions but not API keys (see
/// `users_set_password`'s doc comment) — an intruder who minted a `pw_…` key
/// from a stolen session survives the reset indefinitely. When the target
/// still has at least one key afterward, the admin page must flash a warning
/// naming that residual access instead of leaving the gap silent.
#[tokio::test]
async fn password_reset_flashes_a_warning_when_target_has_api_keys() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("original").unwrap();
    let dave_id = store
        .create_user("dave", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    let (_full, prefix, hash) = pingward::apikey::generate_api_key();
    store
        .insert_api_key(dave_id, "ci", &hash, &prefix, None, chrono::Utc::now())
        .await
        .unwrap();

    let res = server
        .post(&format!("/admin/users/{dave_id}/password"))
        .form(&[("password", "a brand new passphrase")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let flash = res.maybe_cookie("pingward_flash");
    let flash_value = flash.map(|c| c.value().to_string());
    // The `:` separators come back percent-encoded (`%3A`) on the wire, same
    // as the other flash surfaces' cookie values — decode them before checking
    // the signature, which is taken over the decoded payload.
    let payload = flash_value
        .as_deref()
        .map(|v| v.replace("%3A", ":"))
        .and_then(|v| common::flash_payload(&v));
    assert!(
        payload
            .as_deref()
            .is_some_and(|p| p.starts_with("password_reset_keys:")),
        "the flash must carry a signed password_reset_keys payload: {flash_value:?}"
    );

    let body = server.get("/admin").await.text();
    assert!(
        body.contains("data-testid=\"password-reset-flash\""),
        "{body}"
    );
    assert!(body.contains("1 API key"), "{body}");

    // One-shot: a second render must not repeat it.
    let body2 = server.get("/admin").await.text();
    assert!(
        !body2.contains("data-testid=\"password-reset-flash\""),
        "flash should render once: {body2}"
    );
}

/// The mirror case: a target with no API keys gets no warning at all, so the
/// ordinary success path stays quiet as before.
#[tokio::test]
async fn password_reset_has_no_warning_when_target_has_no_api_keys() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("original").unwrap();
    let dave_id = store
        .create_user("dave", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();

    let res = server
        .post(&format!("/admin/users/{dave_id}/password"))
        .form(&[("password", "a brand new passphrase")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(res.maybe_cookie("pingward_flash").is_none());

    let body = server.get("/admin").await.text();
    assert!(
        !body.contains("data-testid=\"password-reset-flash\""),
        "{body}"
    );
}

/// An expired key is already dead — `Store::validate_api_key` refuses it —
/// so it must not inflate the flash's count of keys that "continue to work".
/// The target here has one expired key and one live one; the flash must
/// report only the live one.
#[tokio::test]
async fn password_reset_flash_excludes_expired_api_keys_from_the_count() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("original").unwrap();
    let dave_id = store
        .create_user("dave", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    let (_full, prefix, hash) = pingward::apikey::generate_api_key();
    store
        .insert_api_key(
            dave_id,
            "expired",
            &hash,
            &prefix,
            Some(chrono::Utc::now() - chrono::Duration::hours(1)),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let (_full2, prefix2, hash2) = pingward::apikey::generate_api_key();
    store
        .insert_api_key(dave_id, "live", &hash2, &prefix2, None, chrono::Utc::now())
        .await
        .unwrap();

    let res = server
        .post(&format!("/admin/users/{dave_id}/password"))
        .form(&[("password", "a brand new passphrase")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);

    let body = server.get("/admin").await.text();
    assert!(
        body.contains("data-testid=\"password-reset-flash\""),
        "{body}"
    );
    assert!(body.contains("1 API key that continues"), "{body}");
}

/// The mirror case: when the target's *only* key is already expired, the
/// live count is zero and the flash must not appear at all.
#[tokio::test]
async fn password_reset_has_no_warning_when_only_key_is_expired() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("original").unwrap();
    let dave_id = store
        .create_user("dave", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    let (_full, prefix, hash) = pingward::apikey::generate_api_key();
    store
        .insert_api_key(
            dave_id,
            "expired",
            &hash,
            &prefix,
            Some(chrono::Utc::now() - chrono::Duration::hours(1)),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let res = server
        .post(&format!("/admin/users/{dave_id}/password"))
        .form(&[("password", "a brand new passphrase")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(res.maybe_cookie("pingward_flash").is_none());

    let body = server.get("/admin").await.text();
    assert!(
        !body.contains("data-testid=\"password-reset-flash\""),
        "{body}"
    );
}

/// A disabled target's keys are already inert — `api::extract::ApiUser`
/// re-checks `disabled` on every request — so the flash must not claim
/// residual access, even though the key itself is still live (not expired).
#[tokio::test]
async fn password_reset_has_no_warning_when_target_is_disabled() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("original").unwrap();
    let dave_id = store
        .create_user("dave", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    let (_full, prefix, hash) = pingward::apikey::generate_api_key();
    store
        .insert_api_key(dave_id, "ci", &hash, &prefix, None, chrono::Utc::now())
        .await
        .unwrap();
    store.set_user_disabled(dave_id, true).await.unwrap();

    let res = server
        .post(&format!("/admin/users/{dave_id}/password"))
        .form(&[("password", "a brand new passphrase")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(res.maybe_cookie("pingward_flash").is_none());

    let body = server.get("/admin").await.text();
    assert!(
        !body.contains("data-testid=\"password-reset-flash\""),
        "{body}"
    );
}

#[tokio::test]
async fn disable_and_enable_member() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("frank", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    // Frank has a session before he's disabled.
    login_as(&store, "frank", "pw").await;
    assert_eq!(
        store
            .list_sessions_for_user(uid, chrono::Utc::now())
            .await
            .unwrap()
            .len(),
        1
    );

    server
        .post(&format!("/admin/users/{uid}/disabled?confirmed=1"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(uid).await.unwrap().unwrap().disabled);
    // OWASP: disabling revokes the existing session immediately.
    assert!(
        store
            .list_sessions_for_user(uid, chrono::Utc::now())
            .await
            .unwrap()
            .is_empty()
    );

    server
        .post(&format!("/admin/users/{uid}/disabled?confirmed=1"))
        .await;
    assert!(!store.find_user_by_id(uid).await.unwrap().unwrap().disabled);
    // Core regression: re-enabling must NOT resurrect the old session.
    assert!(
        store
            .list_sessions_for_user(uid, chrono::Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .list_audit(50)
            .await
            .unwrap()
            .iter()
            .any(|a| a.action == "user.set_disabled")
    );
}

#[tokio::test]
async fn deleting_user_cascades_its_sessions() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("grace", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    login_as(&store, "grace", "pw").await;
    assert_eq!(
        store
            .list_sessions_for_user(uid, chrono::Utc::now())
            .await
            .unwrap()
            .len(),
        1
    );

    server
        .post(&format!("/admin/users/{uid}/delete?confirmed=1"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);

    // The `sessions.user_id … ON DELETE CASCADE` FK (plus `PRAGMA foreign_keys
    // = ON`, see `src/db.rs`) must have removed grace's session row — this
    // pins the implicit cascade dependency against a future regression.
    assert!(
        store
            .list_sessions_for_user(uid, chrono::Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn cannot_disable_self() {
    let (server, store, admin_id) = admin_server().await;
    // Only one admin exists here, so this alone can't distinguish the
    // self-guard from the (provably unreachable) last-admin guard — see the
    // comment on `cannot_demote_self` for the same caveat.
    server
        .post(&format!("/admin/users/{admin_id}/disabled?confirmed=1"))
        .await;
    assert!(
        !store
            .find_user_by_id(admin_id)
            .await
            .unwrap()
            .unwrap()
            .disabled
    );
}

/// A flash cookie this origin never signed must not render. Under plain HTTP
/// the `__Host-` prefix is unavailable, so a response from a sibling subdomain
/// can still *write* `pingward_flash` — the signature is what stops the
/// planted value from being read back as a message the server never sent, here
/// a fabricated "99 API keys still work" count on the admin's own page.
#[tokio::test]
async fn a_planted_unsigned_flash_does_not_render() {
    let (mut server, _store, _admin) = admin_server().await;
    server.add_cookie(axum_extra::extract::cookie::Cookie::new(
        "pingward_flash",
        "password_reset_keys:1:99",
    ));

    let body = server.get("/admin").await.text();
    assert!(
        !body.contains("data-testid=\"password-reset-flash\""),
        "an unsigned flash must not render: {body}"
    );
    assert!(!body.contains("99 API keys"), "{body}");
}

/// The same for a fixed-surface flash: `users_blocked` renders a refusal
/// notice that a planted cookie must not be able to fabricate.
#[tokio::test]
async fn a_planted_unsigned_surface_flash_does_not_render() {
    let (mut server, _store, _admin) = admin_server().await;
    server.add_cookie(axum_extra::extract::cookie::Cookie::new(
        "pingward_flash",
        "users_blocked",
    ));

    let body = server.get("/admin").await.text();
    assert!(
        !body.contains("data-testid=\"users-flash\""),
        "an unsigned flash must not render: {body}"
    );
}

/// The reported scenario, end to end: an admin submits the "Add user" form with
/// a username that already exists.
///
/// It used to be a bare `500 internal error` — `users_create` never checked,
/// the `UNIQUE` constraint on `users.username` raised a `sqlx::Error`, and
/// `AppError::Db` rendered a blank page with no message and no form to correct.
#[tokio::test]
async fn creating_a_user_with_a_taken_username_is_refused_with_a_message() {
    let (server, store, _admin) = admin_server().await;
    let before = store.count_users().await.unwrap();

    let res = server
        .post("/admin/users")
        .form(&[("username", "admin"), ("password", "a long enough phrase")])
        .await;

    res.assert_status_ok(); // /admin, re-rendered — not a 500
    let body = res.text();
    assert!(body.contains("user-error"), "{body}");
    assert!(body.contains("already exists"), "{body}");
    assert_eq!(
        store.count_users().await.unwrap(),
        before,
        "nothing may have been created"
    );
    assert!(
        !store
            .list_audit(50)
            .await
            .unwrap()
            .iter()
            .any(|a| a.action == "user.create"),
        "a refused creation is not a creation, and must not be audited as one"
    );
}

/// The existing account is untouched — the reported scenario checked this by
/// hand ("admin's password is still the same"), so it is worth pinning.
#[tokio::test]
async fn a_refused_duplicate_leaves_the_existing_account_alone() {
    let (server, store, admin_id) = admin_server().await;
    let before = store
        .find_user_by_id(admin_id)
        .await
        .unwrap()
        .unwrap()
        .password_hash
        .unwrap();

    server
        .post("/admin/users")
        .form(&[
            ("username", "admin"),
            ("password", "a completely new phrase"),
        ])
        .await;

    let after = store.find_user_by_id(admin_id).await.unwrap().unwrap();
    assert_eq!(after.password_hash.unwrap(), before);
    assert!(after.is_admin, "and is still an admin");
}

/// Exact match, like the constraint: these are two different accounts.
#[tokio::test]
async fn a_username_differing_only_in_case_is_accepted() {
    let (server, store, _admin) = admin_server().await;
    server
        .post("/admin/users")
        .form(&[("username", "Admin"), ("password", "a long enough phrase")])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(
        store
            .find_user_by_username("Admin")
            .await
            .unwrap()
            .is_some()
    );
}
