use chrono::{DateTime, Duration, TimeZone, Utc};
use pingward::{
    db,
    models::{ChannelKind, CheckStatus, NotifyStatus, ScheduleKind},
    notify::{RetryPolicy, deliver_event},
    scheduler::{run_scan_loop, scan_once},
    shutdown,
    store::{NewCheck, Store},
};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_BASE_URL: &str = "https://pingward.test";

async fn empty_store() -> Store {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    sqlx::query("INSERT INTO users (username,is_admin,created_at) VALUES ('u',0,datetime('now'))")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO projects (user_id,name,created_at) VALUES (1,'p',datetime('now'))")
        .execute(&pool)
        .await
        .unwrap();
    Store::new(pool)
}

async fn store_with_up_check(period: i64, grace: i64, last_ping_ago: i64) -> (Store, i64) {
    let store = empty_store().await;
    let id = store
        .create_check(&NewCheck {
            project_id: 1,
            name: "job",
            ping_uuid: "u1",
            kind: ScheduleKind::Period,
            period_secs: Some(period),
            grace_secs: grace,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    let last = Utc::now() - Duration::seconds(last_ping_ago);
    store
        .mark_ping(id, CheckStatus::Up, Some(last), None, None)
        .await
        .unwrap();
    (store, id)
}

async fn store_with_up_check_at(
    period: i64,
    grace: i64,
    last_ping_at: DateTime<Utc>,
) -> (Store, i64) {
    let store = empty_store().await;
    let id = store
        .create_check(&NewCheck {
            project_id: 1,
            name: "job",
            ping_uuid: "u1",
            kind: ScheduleKind::Period,
            period_secs: Some(period),
            grace_secs: grace,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    store
        .mark_ping(id, CheckStatus::Up, Some(last_ping_at), None, None)
        .await
        .unwrap();
    (store, id)
}

#[tokio::test]
async fn overdue_check_transitions_to_down_and_emits_event() {
    // 60 + 30 = 90s window; last ping 200s ago.
    let (store, id) = store_with_up_check(60, 30, 200).await;
    let events = scan_once(&store, Utc::now(), TEST_BASE_URL).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        store
            .find_check_by_uuid("u1")
            .await
            .unwrap()
            .unwrap()
            .status,
        CheckStatus::Down
    );
    let _ = id;
}

#[tokio::test]
async fn healthy_check_is_not_downed() {
    let (store, _) = store_with_up_check(60, 30, 10).await;
    let events = scan_once(&store, Utc::now(), TEST_BASE_URL).await.unwrap();
    assert!(events.is_empty());
    assert_eq!(
        store
            .find_check_by_uuid("u1")
            .await
            .unwrap()
            .unwrap()
            .status,
        CheckStatus::Up
    );
}

#[tokio::test]
async fn scan_once_is_idempotent() {
    let (store, _id) = store_with_up_check(60, 30, 200).await;
    let now = Utc::now();

    let events = scan_once(&store, now, TEST_BASE_URL).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        store
            .find_check_by_uuid("u1")
            .await
            .unwrap()
            .unwrap()
            .status,
        CheckStatus::Down
    );

    // Already Down, so excluded from `list_active_checks`: no second event.
    let events = scan_once(&store, now, TEST_BASE_URL).await.unwrap();
    assert!(events.is_empty());
    assert_eq!(
        store
            .find_check_by_uuid("u1")
            .await
            .unwrap()
            .unwrap()
            .status,
        CheckStatus::Down
    );
}

#[tokio::test]
async fn scan_once_downs_check_exactly_at_due_boundary() {
    let t0 = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
    let due = t0 + Duration::seconds(90);
    let (store, _id) = store_with_up_check_at(60, 30, t0).await;

    // Pins `now >= due`, not `>`.
    let events = scan_once(&store, due, TEST_BASE_URL).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(
        store
            .find_check_by_uuid("u1")
            .await
            .unwrap()
            .unwrap()
            .status,
        CheckStatus::Down
    );
}

#[tokio::test]
async fn scan_once_does_not_down_check_one_second_before_due() {
    let t0 = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
    let due = t0 + Duration::seconds(90);
    let (store, _id) = store_with_up_check_at(60, 30, t0).await;

    let events = scan_once(&store, due - Duration::seconds(1), TEST_BASE_URL)
        .await
        .unwrap();
    assert!(events.is_empty());
    assert_eq!(
        store
            .find_check_by_uuid("u1")
            .await
            .unwrap()
            .unwrap()
            .status,
        CheckStatus::Up
    );
}

#[tokio::test]
async fn overdue_downs_and_delivers_to_bound_channel() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&mock)
        .await;

    let (store, id) = store_with_up_check(60, 30, 200).await;
    let now = Utc::now();
    let cid = store
        .create_channel(
            1,
            ChannelKind::Webhook,
            "hook",
            &format!("{{\"url\":\"{}\"}}", mock.uri()),
            now,
        )
        .await
        .unwrap();
    store.bind_channel(id, cid).await.unwrap();

    let events = scan_once(&store, now, TEST_BASE_URL).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].check_id, id);
    for ev in &events {
        deliver_event(&store, ev, RetryPolicy::default(), now, None).await;
    }
    assert_eq!(
        store.list_recent_notifications(id, 10).await.unwrap()[0].status,
        NotifyStatus::Ok
    );
}

/// The loop's publish site; `tests/sse.rs` covers only the ping side.
#[tokio::test]
async fn run_scan_loop_publishes_down_transition_to_live_tail() {
    let (store, id) = store_with_up_check(60, 30, 200).await;

    // Subscribe before spawning: publishing is gated on `receiver_count() > 0`.
    let (tx, mut rx) = tokio::sync::broadcast::channel(16);
    // Keep `shutdown_tx` alive: dropping it requests shutdown.
    let (_shutdown_tx, shutdown) = shutdown::channel();
    let handle = tokio::spawn(run_scan_loop(
        store.clone(),
        1,
        None,
        TEST_BASE_URL.to_string(),
        tx,
        shutdown,
    ));

    let received = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for the live-tail signal from run_scan_loop")
        .expect("live-tail channel closed unexpectedly");
    assert_eq!(received, id);

    handle.abort();
}

/// `main` joins the loop before closing the pool, so it must *return*.
#[tokio::test]
async fn run_scan_loop_returns_on_shutdown() {
    // 3600s interval: without the `select!` on shutdown it would outlast the timeout.
    let (store, _id) = store_with_up_check(60, 30, 200).await;
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let (shutdown_tx, shutdown) = shutdown::channel();
    let handle = tokio::spawn(run_scan_loop(
        store.clone(),
        3600,
        None,
        TEST_BASE_URL.to_string(),
        tx,
        shutdown,
    ));

    shutdown_tx.trigger();

    tokio::time::timeout(std::time::Duration::from_secs(5), handle)
        .await
        .expect("run_scan_loop must return promptly after shutdown is triggered")
        .expect("run_scan_loop must return normally, not panic");
}
