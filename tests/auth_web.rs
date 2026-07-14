use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

async fn server() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
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
    let (server, store) = server().await;

    // With no users, root redirects to /setup.
    let res = server.get("/").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/setup");

    // Create the first admin.
    let res = server
        .post("/setup")
        .form(&[("username", "admin"), ("password", "pw12345")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(store.count_users().await.unwrap(), 1);
    let admin = store.find_user_by_username("admin").await.unwrap().unwrap();
    assert!(admin.is_admin);

    // Now authenticated (cookie saved) — dashboard renders 200.
    server.get("/").await.assert_status_ok();
}

async fn logged_in_server() -> (TestServer, Store, i64) {
    let (server, store) = server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", "admin"), ("password", "pw")])
        .await;
    (server, store, uid)
}

#[tokio::test]
async fn create_and_delete_project() {
    let (server, store, uid) = logged_in_server().await;

    let res = server
        .post("/projects")
        .form(&[
            ("name", "web"),
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
        .create_project(other, "secret", None, None, chrono::Utc::now())
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
        .create_project(uid, "web", None, None, chrono::Utc::now())
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
    let cfg = Config::from_map(|k| match k {
        "PINGWARD_SMTP_HOST" => Some("mail.example.com".into()),
        "PINGWARD_SMTP_FROM" => Some("alerts@example.com".into()),
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
    server
        .post("/login")
        .form(&[("username", "admin"), ("password", "pw")])
        .await;
    let pid = store
        .create_project(uid, "p", None, None, chrono::Utc::now())
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
        .create_check(
            pid,
            "job",
            "old-uuid",
            pingward::models::ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
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
async fn login_logout_cycle() {
    let (server, store) = server().await;
    let phc = pingward::auth::hash_password("secret1").unwrap();
    store
        .create_user("bob", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();

    // wrong password → back to login with 200 + error
    server
        .post("/login")
        .form(&[("username", "bob"), ("password", "nope")])
        .await
        .assert_status_ok();

    // right password → redirect, cookie set
    let res = server
        .post("/login")
        .form(&[("username", "bob"), ("password", "secret1")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    server.get("/").await.assert_status_ok();

    // logout → redirect, then root bounces to /login
    server
        .post("/logout")
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    let res = server.get("/").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/login");
}

#[tokio::test]
async fn admin_sets_global_scan_interval() {
    let (server, store, _uid) = logged_in_server().await; // admin
    server.get("/settings").await.assert_status_ok();
    server
        .post("/settings")
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
    server.get("/settings").await.assert_status_ok();
    server
        .post("/settings")
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
async fn non_admin_forbidden_from_settings() {
    let (server, store) = server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("plain", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", "plain"), ("password", "pw")])
        .await;
    server
        .get("/settings")
        .await
        .assert_status(axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_creates_and_deletes_user() {
    let (server, store, _uid) = logged_in_server().await;
    server
        .post("/users")
        .form(&[("username", "carol"), ("password", "pw2"), ("is_admin", "")])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    let carol = store.find_user_by_username("carol").await.unwrap().unwrap();
    assert!(!carol.is_admin);
    server
        .post(&format!("/users/{}/delete", carol.id))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store
        .find_user_by_username("carol")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn create_channel_and_bind_to_check() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(
            pid,
            "job",
            "cu",
            pingward::models::ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
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
    assert!(store
        .list_channels_for_project(pid)
        .await
        .unwrap()
        .is_empty());
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
        .create_project(other, "secret", None, None, now)
        .await
        .unwrap();
    let ocid = store
        .create_check(
            opid,
            "j",
            "other-uuid",
            pingward::models::ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
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
        .post(&format!("/users/{uid}/delete"))
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
async fn settings_and_users_pages_use_restyled_field_class() {
    let (server, _store, _uid) = logged_in_server().await; // admin

    let settings_res = server.get("/settings").await;
    settings_res.assert_status_ok();
    assert!(settings_res.text().contains("class=\"field\""));

    let users_res = server.get("/users").await;
    users_res.assert_status_ok();
    assert!(users_res.text().contains("class=\"field\""));
}

#[tokio::test]
async fn check_page_shows_notification_channel_and_error() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(
            pid,
            "job",
            "cu",
            pingward::models::ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
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
