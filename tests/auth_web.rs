use axum_test::TestServer;
use pingward::{app, db, state::AppState, store::Store};

mod common;

/// After a session exists, send its CSRF token as a default `X-CSRF-Token`
/// header so protected POSTs pass `csrf_guard`. Call after every (re)login.
async fn set_csrf(server: &mut TestServer, store: &Store) {
    let tok = common::newest_session_csrf(&store.pool).await;
    server.add_header("x-csrf-token", tok.as_str());
}

async fn server() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    // axum-test 21's `TestServer::new` returns `Self` directly (it panics
    // internally on failure rather than returning a `Result`), matching the
    // note in `tests/ping_api.rs`.
    let mut server = TestServer::new(app(state));
    // axum-test 21 names this `save_cookies` (the brief's `do_save_cookies`
    // does not exist on `TestServer` — that name is used by `TestRequest`
    // instead). Persists Set-Cookie between requests.
    server.save_cookies();
    (server, store)
}

#[tokio::test]
async fn setup_creates_admin_then_dashboard_loads() {
    let (mut server, store) = server().await;

    // With no users, root redirects to /setup.
    let res = server.get("/").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/setup");

    // Create the first admin.
    let csrf = common::anonymous_csrf(&mut server).await;
    let res = server
        .post("/setup")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "admin"),
            ("password", "pw12345"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(store.count_users().await.unwrap(), 1);
    let admin = store.find_user_by_username("admin").await.unwrap().unwrap();
    assert!(admin.is_admin);

    // Now authenticated (cookie saved) — dashboard renders 200.
    server.get("/").await.assert_status_ok();
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
    set_csrf(&mut server, &store).await;
    (server, store, uid)
}

/// P1-D: an active session's idle timer slides forward on use rather than
/// counting down to a fixed cutoff. Time-independent by construction — rather
/// than waiting out a real TTL, an existing session's `expires_at` is moved
/// to just shy of expiry directly in the database, then one authenticated
/// request is issued and the row is re-read.
#[tokio::test]
async fn an_active_session_survives_past_the_original_ttl() {
    let (server, store, _uid) = logged_in_server().await;

    let session_id: String =
        sqlx::query_scalar::<_, String>("SELECT id FROM sessions ORDER BY rowid DESC LIMIT 1")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    let near_expiry = chrono::Utc::now() + chrono::Duration::minutes(1);
    sqlx::query("UPDATE sessions SET expires_at = $1 WHERE id = $2")
        .bind(near_expiry.to_rfc3339())
        .bind(&session_id)
        .execute(&store.pool)
        .await
        .unwrap();

    // One authenticated request — this is what triggers the slide.
    server.get("/").await.assert_status_ok();

    let expires_at: String =
        sqlx::query_scalar::<_, String>("SELECT expires_at FROM sessions WHERE id = $1")
            .bind(&session_id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
    let expires_at = chrono::DateTime::parse_from_rfc3339(&expires_at)
        .unwrap()
        .with_timezone(&chrono::Utc);
    assert!(
        expires_at > near_expiry + chrono::Duration::hours(1),
        "expires_at must have slid forward past its near-expiry value, got {expires_at}"
    );
}

#[tokio::test]
async fn disabling_user_invalidates_session() {
    let (server, store, uid) = logged_in_server().await;
    // Authenticated: dashboard is 200.
    server.get("/").await.assert_status_ok();
    // Disable the account (mirroring what `web::users_set_disabled` does: set
    // the flag, then revoke sessions — this test bypasses the HTTP route
    // itself because the real route refuses to let an admin disable
    // themselves, and `uid` here is the sole admin), then the same session
    // must redirect to /login.
    store.set_user_disabled(uid, true).await.unwrap();
    store.delete_sessions_for_user(uid).await.unwrap();
    let res = server.get("/projects/new").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/login");
    // OWASP: the session row itself must be gone, not merely rejected by the
    // `resolve_user` disabled check.
    assert!(
        store
            .list_sessions_for_user(uid, chrono::Utc::now())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn disabled_user_cannot_log_in() {
    let (mut server, store) = server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("bob", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    store.set_user_disabled(uid, true).await.unwrap();
    let csrf = common::anonymous_csrf(&mut server).await;
    let res = server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "bob"),
            ("password", "pw"),
        ])
        .await;
    // Login page re-renders with an error (200), no session cookie set.
    res.assert_status_ok();
}

#[tokio::test]
async fn create_and_delete_project() {
    let (server, store, uid) = logged_in_server().await;

    let res = server
        .post("/projects")
        .form(&[
            ("name", "web"),
            ("description", ""),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let projects = store.list_projects_for_user(uid).await.unwrap();
    assert_eq!(projects.len(), 1);
    let pid = projects[0].id;

    server
        .get(&format!("/projects/{pid}"))
        .await
        .assert_status_ok();

    server
        .post(&format!("/projects/{pid}/delete"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.list_projects_for_user(uid).await.unwrap().is_empty());
}

#[tokio::test]
async fn cannot_view_another_users_project() {
    let (server, store, _uid) = logged_in_server().await;
    // project owned by a different user
    let other = store
        .create_user("other", Some("x"), false, chrono::Utc::now())
        .await
        .unwrap();
    let pid = store
        .create_project(other, "secret", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    server
        .get(&format!("/projects/{pid}"))
        .await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
}

async fn server_with_project() -> (TestServer, Store, i64) {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    (server, store, pid)
}

async fn server_with_project_and_smtp() -> (TestServer, Store, i64) {
    use pingward::{app, config::Config, state::AppState, store::Store};
    let pool = pingward::db::connect("sqlite::memory:").await.unwrap();
    pingward::db::migrate(&pool, "sqlite::memory:")
        .await
        .unwrap();
    let store = Store::new(pool);
    // Pins the same secret `common::test_config` uses, so `set_csrf` derives a
    // token this server accepts — an unpinned secret is random per call.
    let cfg = Config::from_map(|k| match k {
        "PINGWARD_SMTP_HOST" => Some("mail.example.com".into()),
        "PINGWARD_SMTP_FROM" => Some("alerts@example.com".into()),
        "PINGWARD_SECRET" => Some(common::TEST_SECRET.into()),
        _ => None,
    });
    let state = AppState::new(store.clone(), cfg);
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
    set_csrf(&mut server, &store).await;
    let pid = store
        .create_project(uid, "p", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    (server, store, pid)
}

#[tokio::test]
async fn channel_form_hides_email_without_smtp() {
    let (server, _store, pid) = server_with_project().await;
    let res = server.get(&format!("/projects/{pid}/channels/new")).await;
    res.assert_status_ok();
    assert!(
        !res.text().contains("value=\"email\""),
        "email option must be hidden when SMTP is unconfigured"
    );
}

#[tokio::test]
async fn channel_form_shows_email_with_smtp() {
    let (server, _store, pid) = server_with_project_and_smtp().await;
    let res = server.get(&format!("/projects/{pid}/channels/new")).await;
    res.assert_status_ok();
    assert!(
        res.text().contains("value=\"email\""),
        "email option must appear when SMTP is configured"
    );
}

#[tokio::test]
async fn create_email_channel_stores_recipient() {
    let (server, store, pid) = server_with_project_and_smtp().await;
    let res = server
        .post(&format!("/projects/{pid}/channels"))
        .form(&[
            ("name", "ops"),
            ("kind", "email"),
            ("email_to", "ops@example.com"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let channels = store.list_channels_for_project(pid).await.unwrap();
    assert_eq!(channels.len(), 1);
    assert!(channels[0].config_json.contains("ops@example.com"));
}

#[tokio::test]
async fn create_check_and_pause_resume() {
    let (server, store, pid) = server_with_project().await;

    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "backup"),
            ("description", ""),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("grace_secs", "300"),
            ("cron_expr", ""),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let checks = store.list_checks_for_project(pid).await.unwrap();
    assert_eq!(checks.len(), 1);
    let cid = checks[0].id;

    server
        .post(&format!("/checks/{cid}/pause"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().status,
        pingward::models::CheckStatus::Paused
    );

    server
        .post(&format!("/checks/{cid}/resume"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().status,
        pingward::models::CheckStatus::New
    );
}

#[tokio::test]
async fn acknowledge_persists() {
    let (server, store, pid) = server_with_project().await;

    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "backup"),
            ("description", ""),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("grace_secs", "300"),
            ("cron_expr", ""),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let checks = store.list_checks_for_project(pid).await.unwrap();
    let cid = checks[0].id;

    store
        .set_status(cid, pingward::models::CheckStatus::Down)
        .await
        .unwrap();

    server
        .post(&format!("/checks/{cid}/ack"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.find_check(cid).await.unwrap().unwrap().acknowledged);
}

#[tokio::test]
async fn create_check_persists_max_runtime() {
    let (server, store, pid) = server_with_project().await;
    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "job"),
            ("description", ""),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("grace_secs", "300"),
            ("cron_expr", ""),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", "120"),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let checks = store.list_checks_for_project(pid).await.unwrap();
    assert_eq!(checks[0].max_runtime_secs, Some(120));
}

#[tokio::test]
async fn create_check_persists_nag_interval() {
    let (server, store, pid) = server_with_project().await;
    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "job"),
            ("description", ""),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("grace_secs", "300"),
            ("cron_expr", ""),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", "120"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let checks = store.list_checks_for_project(pid).await.unwrap();
    assert_eq!(checks[0].nag_interval_secs, Some(120));
}

#[tokio::test]
async fn invalid_cron_is_rejected() {
    let (server, store, pid) = server_with_project().await;
    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("name", "bad"),
            ("description", ""),
            ("schedule_kind", "cron"),
            ("period_secs", ""),
            ("grace_secs", "60"),
            ("cron_expr", "not a cron"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_ok(); // re-rendered form, not a redirect
    assert!(store.list_checks_for_project(pid).await.unwrap().is_empty());
}

#[tokio::test]
async fn regenerate_uuid_changes_ping_url() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "job",
            ping_uuid: "old-uuid",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    server
        .post(&format!("/checks/{cid}/regenerate"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_ne!(
        store.find_check(cid).await.unwrap().unwrap().ping_uuid,
        "old-uuid"
    );
}

#[tokio::test]
async fn setup_page_uses_auth_card() {
    let (server, _store) = server().await;
    let res = server.get("/setup").await;
    res.assert_status_ok();
    assert!(res.text().contains("class=\"auth\""));
}

#[tokio::test]
async fn login_page_uses_auth_card_and_error_is_restyled() {
    let (mut server, store) = server().await;
    let phc = pingward::auth::hash_password("secret1").unwrap();
    store
        .create_user("bob", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();

    let res = server.get("/login").await;
    res.assert_status_ok();
    assert!(res.text().contains("class=\"auth\""));

    // Wrong password → re-rendered login page, error still reachable.
    let csrf = common::anonymous_csrf(&mut server).await;
    let res = server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "bob"),
            ("password", "nope"),
        ])
        .await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("invalid username or password"), "got: {body}");
    assert!(
        body.contains("class=\"flash err\""),
        "missing restyled err banner: {body}"
    );
}

#[tokio::test]
async fn login_page_bounces_an_already_signed_in_visitor() {
    // Mostly there for forward auth, where `logout` lands on `/login` only to
    // be re-authenticated by the gateway header before the page renders — but
    // the rule is unconditional, so it is testable without a proxy.
    let (mut server, store) = server().await;
    let phc = pingward::auth::hash_password("secret1").unwrap();
    store
        .create_user("bob", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();

    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "bob"),
            ("password", "secret1"),
        ])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);

    let res = server.get("/login").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/");
}

#[tokio::test]
async fn login_logout_cycle() {
    let (mut server, store) = server().await;
    let phc = pingward::auth::hash_password("secret1").unwrap();
    store
        .create_user("bob", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();

    // wrong password → back to login with 200 + error
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "bob"),
            ("password", "nope"),
        ])
        .await
        .assert_status_ok();

    // right password → redirect, cookie set
    let csrf = common::anonymous_csrf(&mut server).await;
    let res = server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "bob"),
            ("password", "secret1"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let login_secure = res
        .cookie(pingward::auth::session_cookie_name(false))
        .secure();
    set_csrf(&mut server, &store).await;
    server.get("/").await.assert_status_ok();

    // logout → redirect, then root bounces to /login
    let logout_res = server.post("/logout").await;
    logout_res.assert_status(axum::http::StatusCode::SEE_OTHER);
    // The removal cookie must carry `Path=/` (matching how the cookie was
    // set) and the same `Secure` attribute as the cookie it clears —
    // otherwise a browser can fail to overwrite/clear it. See
    // `web::session_removal_cookie`.
    let removal_cookie = logout_res.cookie(pingward::auth::session_cookie_name(false));
    assert_eq!(removal_cookie.path(), Some("/"));
    assert_eq!(removal_cookie.secure(), login_secure);
    // A password logout asks the browser to drop this origin's cookies and
    // cache — but never "storage", which would wipe the pw-theme preference.
    let clear_site_data = logout_res
        .header("clear-site-data")
        .to_str()
        .unwrap()
        .to_string();
    assert_eq!(clear_site_data, r#""cache", "cookies""#);
    assert!(!clear_site_data.contains("storage"));
    let res = server.get("/").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/login");
}

/// A server whose `Config` is pinned to [`common::TEST_SECRET`] and, when
/// given, `PINGWARD_BASE_URL` — otherwise built exactly like [`server`].
async fn server_with_base_url(base_url: Option<&str>) -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let config = pingward::config::Config::from_map(|k| match k {
        "PINGWARD_SECRET" => Some(common::TEST_SECRET.into()),
        "PINGWARD_BASE_URL" => base_url.map(str::to_string),
        _ => None,
    });
    let state = AppState::new(store.clone(), config);
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    (server, store)
}

/// Signs `alice` in and returns the login response, whose `Set-Cookie` carries
/// the freshly minted session cookie.
async fn login_alice(server: &mut TestServer, store: &Store) -> axum_test::TestResponse {
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("alice", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    let csrf = common::anonymous_csrf(server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "alice"),
            ("password", "pw"),
        ])
        .await
}

/// The raw `Set-Cookie` header value for the session cookie, attributes and
/// all. `cookie::Cookie::secure()` cannot tell "explicitly not Secure" apart
/// from "Secure never mentioned" once a header round-trips through the
/// parser (an absent flag parses to `None`, not `Some(false)`), so the raw
/// header is the unambiguous way to check both presence and absence.
///
/// Takes `cookie_secure` because the cookie's *name* depends on it (P2-G):
/// `__Host-pingward_session` when Secure is on, `pingward_session` otherwise.
fn raw_session_set_cookie(res: &axum_test::TestResponse, cookie_secure: bool) -> String {
    let name = pingward::auth::session_cookie_name(cookie_secure);
    res.headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with(&format!("{name}=")))
        .unwrap_or_else(|| panic!("a Set-Cookie for {name}"))
        .to_string()
}

/// P0-A: the session cookie's `Secure` attribute follows `PINGWARD_BASE_URL`'s
/// scheme (via `config::parse_cookie_secure`), and it never carries
/// `Max-Age`/`Expires` — a non-persistent cookie is OWASP's stated preference
/// for an authenticated session, enforced server-side via
/// `sessions.expires_at` instead. See `web::session_cookie`.
///
/// P2-G: on the `https://` server, the cookie's name also carries the
/// `__Host-` prefix, and all three of the prefix's browser-enforced
/// conditions hold — `Secure`, `Path=/`, and no `Domain` — so the browser
/// itself refuses to let a sibling subdomain or a downgraded HTTP response
/// overwrite it.
#[tokio::test]
async fn session_cookie_carries_secure_only_when_configured() {
    let (mut https_server, https_store) =
        server_with_base_url(Some("https://pingward.example")).await;
    let res = login_alice(&mut https_server, &https_store).await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let set_cookie = raw_session_set_cookie(&res, true);
    assert!(
        set_cookie.starts_with("__Host-pingward_session="),
        "{set_cookie}"
    );
    assert!(set_cookie.contains("; Secure"), "{set_cookie}");
    assert!(set_cookie.contains("; Path=/"), "{set_cookie}");
    assert!(
        !set_cookie.to_ascii_lowercase().contains("domain="),
        "{set_cookie}"
    );
    assert!(
        !set_cookie.to_ascii_lowercase().contains("max-age"),
        "{set_cookie}"
    );
    assert!(
        !set_cookie.to_ascii_lowercase().contains("expires"),
        "{set_cookie}"
    );

    let (mut http_server, http_store) = server_with_base_url(None).await;
    let res = login_alice(&mut http_server, &http_store).await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let set_cookie = raw_session_set_cookie(&res, false);
    assert!(set_cookie.starts_with("pingward_session="), "{set_cookie}");
    assert!(!set_cookie.contains("; Secure"), "{set_cookie}");
    assert!(
        !set_cookie.to_ascii_lowercase().contains("max-age"),
        "{set_cookie}"
    );
    assert!(
        !set_cookie.to_ascii_lowercase().contains("expires"),
        "{set_cookie}"
    );
}

/// P2-G: the `__Host-` prefixed name is consistent across both the write side
/// (`session_cookie`, exercised by login) and the read side
/// (`secret::session_id_from_jar`, exercised by every authenticated route and
/// by `logout`'s removal cookie) on a `Secure` deployment.
#[tokio::test]
async fn host_prefixed_cookie_round_trips() {
    let (mut server, store) = server_with_base_url(Some("https://pingward.example")).await;
    let res = login_alice(&mut server, &store).await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);

    server.get("/account").await.assert_status_ok();

    set_csrf(&mut server, &store).await;
    server
        .post("/logout")
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
}

/// P1-F: every response from `web::routes()` carries `Cache-Control:
/// no-store`, so an authenticated page, the pre-login CSRF-bearing forms, and
/// even a `csrf_guard` rejection are never cached by the browser, a shared
/// computer, or an intermediary proxy. See `web::no_store`.
#[tokio::test]
async fn browser_responses_are_not_cacheable() {
    let (auth_server, store, uid) = logged_in_server().await;

    let res = auth_server.get("/").await;
    res.assert_status_ok();
    assert_eq!(res.header("cache-control"), "no-store");

    let pid = store
        .create_project(uid, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            ping_uuid: "cu",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    let res = auth_server.get(&format!("/checks/{cid}")).await;
    res.assert_status_ok();
    assert_eq!(res.header("cache-control"), "no-store");

    // A fresh, logged-out server still carries the header on the pre-login
    // forms, which embed a cookie-bound `_csrf`.
    let (logged_out, _store) = server().await;
    let res = logged_out.get("/login").await;
    assert_eq!(res.header("cache-control"), "no-store");

    // A POST rejected by `csrf_guard` (missing/invalid token -> 403) still
    // carries it — `no_store` sits outermost precisely so early returns like
    // this one are covered too.
    let res = logged_out
        .post("/login")
        .form(&[("username", "admin"), ("password", "pw")])
        .await;
    res.assert_status(axum::http::StatusCode::FORBIDDEN);
    assert_eq!(res.header("cache-control"), "no-store");
}

#[tokio::test]
async fn admin_sets_global_scan_interval() {
    let (server, store, _uid) = logged_in_server().await; // admin
    server.get("/admin").await.assert_status_ok();
    server
        .post("/admin/settings")
        .form(&[
            ("scan_interval", "45"),
            ("nag_interval", ""),
            ("pings_retention_days", ""),
            ("notifications_retention_days", ""),
        ])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        store.get_setting("scan_interval").await.unwrap().as_deref(),
        Some("45")
    );
}

#[tokio::test]
async fn admin_sets_retention_days() {
    let (server, store, _uid) = logged_in_server().await; // admin
    server.get("/admin").await.assert_status_ok();
    server
        .post("/admin/settings")
        .form(&[
            ("scan_interval", ""),
            ("nag_interval", ""),
            ("pings_retention_days", "30"),
            ("notifications_retention_days", "90"),
        ])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(
        store
            .get_setting("pings_retention_days")
            .await
            .unwrap()
            .as_deref(),
        Some("30")
    );
    assert_eq!(
        store
            .get_setting("notifications_retention_days")
            .await
            .unwrap()
            .as_deref(),
        Some("90")
    );
}

#[tokio::test]
async fn admin_creates_and_deletes_user() {
    let (server, store, _uid) = logged_in_server().await;
    server
        .post("/admin/users")
        .form(&[("username", "carol"), ("password", "pw2"), ("is_admin", "")])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    let carol = store.find_user_by_username("carol").await.unwrap().unwrap();
    assert!(!carol.is_admin);
    server
        .post(&format!("/admin/users/{}/delete", carol.id))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(
        store
            .find_user_by_username("carol")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn create_channel_and_bind_to_check() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "job",
            ping_uuid: "cu",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    // create a webhook channel
    let res = server
        .post(&format!("/projects/{pid}/channels"))
        .form(&[
            ("name", "hook"),
            ("kind", "webhook"),
            ("webhook_url", "http://example.test/h"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let channels = store.list_channels_for_project(pid).await.unwrap();
    assert_eq!(channels.len(), 1);
    let chid = channels[0].id;
    assert!(channels[0].config_json.contains("example.test"));

    // bind it to the check
    server
        .post(&format!("/checks/{cid}/channels"))
        .form(&[("channel_ids", chid.to_string().as_str())])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(store.bound_channel_ids(cid).await.unwrap(), vec![chid]);

    // unbind by submitting no channel_ids
    server
        .post(&format!("/checks/{cid}/channels"))
        .form(&[("_", "")])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.bound_channel_ids(cid).await.unwrap().is_empty());
}

#[tokio::test]
async fn create_telegram_channel_persists_config() {
    let (server, store, pid) = server_with_project().await;

    let res = server
        .post(&format!("/projects/{pid}/channels"))
        .form(&[
            ("name", "tg"),
            ("kind", "telegram"),
            ("telegram_token", "123:ABC"),
            ("telegram_chat_id", "999"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);

    let channels = store.list_channels_for_project(pid).await.unwrap();
    let tg = channels
        .iter()
        .find(|c| c.kind == pingward::models::ChannelKind::Telegram)
        .expect("telegram channel persisted");
    assert!(tg.config_json.contains("\"token\":\"123:ABC\""));
    assert!(tg.config_json.contains("\"chat_id\":\"999\""));
}

#[tokio::test]
async fn create_slack_channel_persists_config() {
    let (server, store, pid) = server_with_project().await;
    let res = server
        .post(&format!("/projects/{pid}/channels"))
        .form(&[
            ("name", "sl"),
            ("kind", "slack"),
            ("slack_url", "http://hooks.test/x"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);

    let channels = store.list_channels_for_project(pid).await.unwrap();
    let ch = channels
        .iter()
        .find(|c| c.kind == pingward::models::ChannelKind::Slack)
        .expect("slack channel persisted");
    assert!(ch.config_json.contains("hooks.test"));
}

#[tokio::test]
async fn create_ntfy_channel_persists_config() {
    let (server, store, pid) = server_with_project().await;
    let res = server
        .post(&format!("/projects/{pid}/channels"))
        .form(&[
            ("name", "nt"),
            ("kind", "ntfy"),
            ("ntfy_base_url", "https://ntfy.example"),
            ("ntfy_topic", "alerts"),
            ("ntfy_token", ""),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);

    let channels = store.list_channels_for_project(pid).await.unwrap();
    let ch = channels
        .iter()
        .find(|c| c.kind == pingward::models::ChannelKind::Ntfy)
        .expect("ntfy channel persisted");
    assert!(ch.config_json.contains("\"topic\":\"alerts\""));
    assert!(ch.config_json.contains("ntfy.example"));
}

#[tokio::test]
async fn create_pushover_channel_persists_config() {
    let (server, store, pid) = server_with_project().await;
    let res = server
        .post(&format!("/projects/{pid}/channels"))
        .form(&[
            ("name", "po"),
            ("kind", "pushover"),
            ("pushover_token", "apptok"),
            ("pushover_user", "userkey"),
        ])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);

    let channels = store.list_channels_for_project(pid).await.unwrap();
    let ch = channels
        .iter()
        .find(|c| c.kind == pingward::models::ChannelKind::Pushover)
        .expect("pushover channel persisted");
    assert!(ch.config_json.contains("\"token\":\"apptok\""));
    assert!(ch.config_json.contains("\"user\":\"userkey\""));
}

#[tokio::test]
async fn channel_create_rejects_blank_required_field() {
    let (server, store, pid) = server_with_project().await;
    // telegram with a blank chat_id → re-rendered form (200), nothing persisted.
    let res = server
        .post(&format!("/projects/{pid}/channels"))
        .form(&[
            ("name", "tg"),
            ("kind", "telegram"),
            ("telegram_token", "123:ABC"),
            ("telegram_chat_id", ""),
        ])
        .await;
    res.assert_status_ok();
    assert!(
        store
            .list_channels_for_project(pid)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Set up a second user owning a project + check + channel, for authorization
/// negative-path tests run as the logged-in `admin`.
async fn other_users_project(store: &Store) -> (i64, i64, i64) {
    let now = chrono::Utc::now();
    let other = store
        .create_user("other", Some("x"), false, now)
        .await
        .unwrap();
    let opid = store
        .create_project(other, "secret", "", None, None, now)
        .await
        .unwrap();
    let ocid = store
        .create_check(&pingward::store::NewCheck {
            project_id: opid,
            name: "j",
            ping_uuid: "other-uuid",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    let ochid = store
        .create_channel(
            opid,
            pingward::models::ChannelKind::Webhook,
            "h",
            "{\"url\":\"http://other.test/h\"}",
            now,
        )
        .await
        .unwrap();
    (opid, ocid, ochid)
}

#[tokio::test]
async fn cannot_operate_on_another_users_check() {
    let (server, store, _uid) = logged_in_server().await;
    let (_opid, ocid, _ochid) = other_users_project(&store).await;

    server
        .get(&format!("/checks/{ocid}"))
        .await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
    server
        .post(&format!("/checks/{ocid}/pause"))
        .await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
    server
        .post(&format!("/checks/{ocid}/delete"))
        .await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
    // The check must still exist — no cross-user mutation happened.
    assert!(store.find_check(ocid).await.unwrap().is_some());
}

#[tokio::test]
async fn non_owner_cannot_acknowledge() {
    let (server, store, _uid) = logged_in_server().await;
    let (_opid, ocid, _ochid) = other_users_project(&store).await;
    store
        .set_status(ocid, pingward::models::CheckStatus::Down)
        .await
        .unwrap();

    server
        .post(&format!("/checks/{ocid}/ack"))
        .await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
    // No cross-user mutation happened.
    assert!(!store.find_check(ocid).await.unwrap().unwrap().acknowledged);
}

#[tokio::test]
async fn cannot_delete_another_users_channel() {
    let (server, store, _uid) = logged_in_server().await;
    let (_opid, _ocid, ochid) = other_users_project(&store).await;

    server
        .post(&format!("/channels/{ochid}/delete"))
        .await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
    assert!(store.find_channel(ochid).await.unwrap().is_some());
}

#[tokio::test]
async fn cannot_create_channel_in_another_users_project() {
    let (server, store, _uid) = logged_in_server().await;
    let (opid, _ocid, _ochid) = other_users_project(&store).await;

    server
        .post(&format!("/projects/{opid}/channels"))
        .form(&[
            ("name", "x"),
            ("kind", "webhook"),
            ("webhook_url", "http://evil.test/h"),
        ])
        .await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
    // Only the other user's own channel remains; nothing was injected.
    let channels = store.list_channels_for_project(opid).await.unwrap();
    assert_eq!(channels.len(), 1);
    assert!(channels[0].config_json.contains("other.test"));
}

#[tokio::test]
async fn admin_cannot_delete_self() {
    let (server, store, uid) = logged_in_server().await; // uid is the sole admin
    server
        .post(&format!("/admin/users/{uid}/delete"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    // Self-delete is a no-op guard: the admin must still exist.
    assert!(store.find_user_by_id(uid).await.unwrap().is_some());
}

#[tokio::test]
async fn send_test_notification_reports_success() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    let (server, store, pid) = server_with_project().await;
    let chid = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook",
            &format!("{{\"url\":\"{}\"}}", mock.uri()),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let res = server.post(&format!("/channels/{chid}/test")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("Test notification sent"), "got: {body}");
    assert!(
        body.contains("class=\"flash ok\""),
        "missing restyled ok banner: {body}"
    );
}

#[tokio::test]
async fn send_test_notification_reports_failure() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;

    let (server, store, pid) = server_with_project().await;
    let chid = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook",
            &format!("{{\"url\":\"{}\"}}", mock.uri()),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let res = server.post(&format!("/channels/{chid}/test")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("Test notification failed"), "got: {body}");
    assert!(
        body.contains("class=\"flash err\""),
        "missing restyled err banner: {body}"
    );
}

#[tokio::test]
async fn admin_page_uses_restyled_field_class() {
    let (server, _store, _uid) = logged_in_server().await; // admin

    // Settings and add-user are now sections of the same merged /admin page.
    let res = server.get("/admin").await;
    res.assert_status_ok();
    assert!(res.text().contains("class=\"field\""));
}

#[tokio::test]
async fn check_page_shows_notification_channel_and_error() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "job",
            ping_uuid: "cu",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    let chid = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "my-hook",
            "{\"url\":\"http://x\"}",
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    store
        .record_notification(
            cid,
            chid,
            pingward::notify::EventKind::Down,
            pingward::models::NotifyStatus::Error,
            Some("status 500"),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let res = server.get(&format!("/checks/{cid}")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("my-hook"), "channel name missing: {body}");
    assert!(body.contains("status 500"), "error text missing: {body}");
}
