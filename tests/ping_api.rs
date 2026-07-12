use axum_test::TestServer;
use pingward::{app, config::Config, db, models::ScheduleKind, state::AppState, store::Store};
use sqlx::Row;

async fn test_server() -> (TestServer, Store) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO users (username, is_admin, created_at) VALUES ('u',0,datetime('now'))",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO projects (user_id, name, created_at) VALUES (1,'p',datetime('now'))")
        .execute(&pool)
        .await
        .unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    // axum-test 21's `TestServer::new` returns `Self` directly (it panics
    // internally on failure rather than returning a `Result`), unlike the
    // brief's `.unwrap()` which assumed a `Result`.
    let server = TestServer::new(app(state));
    (server, store)
}

#[tokio::test]
async fn healthz_returns_ok() {
    let (server, _) = test_server().await;
    server.get("/healthz").await.assert_status_ok();
}

#[tokio::test]
async fn success_ping_marks_up_and_records() {
    let (server, store) = test_server().await;
    store
        .create_check(
            1,
            "job",
            "abc",
            ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();

    server
        .post("/ping/abc")
        .text("done")
        .await
        .assert_status_ok();

    let c = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(c.status, pingward::models::CheckStatus::Up);
    assert!(c.last_ping_at.is_some());
    assert!(c.next_due_at.is_some());
}

#[tokio::test]
async fn fail_ping_marks_down() {
    let (server, store) = test_server().await;
    store
        .create_check(
            1,
            "job",
            "abc",
            ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();
    server.post("/ping/abc/fail").await.assert_status_ok();
    let c = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(c.status, pingward::models::CheckStatus::Down);
}

#[tokio::test]
async fn unknown_uuid_is_404() {
    let (server, _) = test_server().await;
    server
        .get("/ping/does-not-exist")
        .await
        .assert_status(axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn exit_code_nonzero_marks_down() {
    let (server, store) = test_server().await;
    store
        .create_check(
            1,
            "job",
            "abc",
            ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();
    server.post("/ping/abc/1").await.assert_status_ok();
    let c = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(c.status, pingward::models::CheckStatus::Down);
}

#[tokio::test]
async fn start_does_not_reset_due_clock() {
    let (server, store) = test_server().await;
    store
        .create_check(
            1,
            "job",
            "abc",
            ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();

    server.post("/ping/abc").await.assert_status_ok();
    let before = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(before.status, pingward::models::CheckStatus::Up);
    assert!(before.last_ping_at.is_some());
    assert!(before.next_due_at.is_some());

    server.post("/ping/abc/start").await.assert_status_ok();
    let after = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(after.status, pingward::models::CheckStatus::Up);
    assert_eq!(after.last_ping_at, before.last_ping_at);
    assert_eq!(after.next_due_at, before.next_due_at);
    assert!(after.last_start_at.is_some());
}

#[tokio::test]
async fn log_records_only_no_state_change() {
    let (server, store) = test_server().await;
    store
        .create_check(
            1,
            "job",
            "abc",
            ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();

    let before = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(before.status, pingward::models::CheckStatus::New);

    server.post("/ping/abc/log").await.assert_status_ok();

    let after = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(after.status, pingward::models::CheckStatus::New);
    assert!(after.last_ping_at.is_none());
}

#[tokio::test]
async fn exit_code_zero_marks_up() {
    let (server, store) = test_server().await;
    store
        .create_check(
            1,
            "job",
            "abc",
            ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();
    server.post("/ping/abc/0").await.assert_status_ok();
    let c = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(c.status, pingward::models::CheckStatus::Up);
    assert!(c.last_ping_at.is_some());
}

#[tokio::test]
async fn get_verb_works_for_success() {
    let (server, store) = test_server().await;
    store
        .create_check(
            1,
            "job",
            "abc",
            ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();
    server.get("/ping/abc").await.assert_status_ok();
    let c = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(c.status, pingward::models::CheckStatus::Up);
}

/// Spec §6: a paused check is excluded from monitoring, so a ping must not
/// resurrect it into `up`/`down`. The ping is still recorded for history.
#[tokio::test]
async fn paused_check_is_not_resurrected_by_a_ping() {
    let (server, store) = test_server().await;
    let id = store
        .create_check(
            1,
            "job",
            "abc",
            ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();
    store
        .set_status(id, pingward::models::CheckStatus::Paused)
        .await
        .unwrap();

    server
        .post("/ping/abc")
        .text("done")
        .await
        .assert_status_ok();

    let c = store.find_check_by_uuid("abc").await.unwrap().unwrap();
    assert_eq!(c.status, pingward::models::CheckStatus::Paused);

    let row = sqlx::query("SELECT COUNT(*) AS cnt FROM pings WHERE check_id = ?")
        .bind(id)
        .fetch_one(&store.pool)
        .await
        .unwrap();
    let cnt: i64 = row.get("cnt");
    assert_eq!(cnt, 1, "ping should still be recorded even while paused");
}
