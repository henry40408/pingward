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
