use axum_test::TestServer;
use pingward::{app, db, models::ScheduleKind, store::Store};

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
    // axum-test 21's `TestServer::new` returns `Self` directly (it panics
    // internally on failure rather than returning a `Result`), unlike the
    // brief's `.unwrap()` which assumed a `Result`.
    let server = TestServer::new(app(store.clone()));
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
