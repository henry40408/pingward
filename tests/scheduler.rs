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

/// Notifications render their check links against this.
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

/// An Up check with a fixed `last_ping_at`, for boundary control.
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
    // period 60 + grace 30 = 90s; last ping 200s ago → overdue
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
    // last ping 10s ago, window 90s → healthy
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
    // period 60 + grace 30 = 90s; last ping 200s ago → overdue
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
    // period 60 + grace 30 = 90s; due = t0 + 90s.
    let t0 = Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap();
    let due = t0 + Duration::seconds(90);
    let (store, _id) = store_with_up_check_at(60, 30, t0).await;

    // now == due exactly: the comparison is `>=`, so this must down the check.
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
    // period 60 + grace 30 = 90s; due = t0 + 90s.
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

/// Covers the loop's own publish site: `run_scan_loop` publishes each `Down`
/// transition's `check_id` to the live-tail bus, which `tests/sse.rs` (the
/// ping-side publish) does not exercise.
#[tokio::test]
async fn run_scan_loop_publishes_down_transition_to_live_tail() {
    // 90s window, last ping 200s ago → overdue on the loop's first pass.
    let (store, id) = store_with_up_check(60, 30, 200).await;

    // Subscribe before spawning: the publish is gated on `receiver_count() > 0`,
    // so subscribing later is a race against the loop's first pass.
    let (tx, mut rx) = tokio::sync::broadcast::channel(16);
    // Hold `shutdown_tx`: dropping it is itself a shutdown request, which would
    // end the loop early.
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

    // The loop runs forever until shutdown; don't let it outlive the test.
    handle.abort();
}

/// The loop ends by *returning*: `handle.await` yielding `Ok(())` proves it ran
/// to completion rather than being aborted, which is what `main` relies on to
/// close the pool with no query in flight.
#[tokio::test]
async fn run_scan_loop_returns_on_shutdown() {
    // A long interval: without the select on `shutdown`, the loop would sit in
    // `sleep` far past this test's timeout.
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
