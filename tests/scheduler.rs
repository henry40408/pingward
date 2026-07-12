use chrono::{DateTime, Duration, TimeZone, Utc};
use pingward::{
    db,
    models::{ChannelKind, CheckStatus, NotifyStatus, ScheduleKind},
    notify::{deliver_event, RetryPolicy},
    scheduler::scan_once,
    store::Store,
};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

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
        .create_check(
            1,
            "job",
            "u1",
            ScheduleKind::Period,
            Some(period),
            grace,
            None,
            "UTC",
        )
        .await
        .unwrap();
    let last = Utc::now() - Duration::seconds(last_ping_ago);
    store
        .mark_ping(id, CheckStatus::Up, Some(last), None, None)
        .await
        .unwrap();
    (store, id)
}

/// Seeds an Up check with a FIXED last_ping_at for precise boundary control.
async fn store_with_up_check_at(
    period: i64,
    grace: i64,
    last_ping_at: DateTime<Utc>,
) -> (Store, i64) {
    let store = empty_store().await;
    let id = store
        .create_check(
            1,
            "job",
            "u1",
            ScheduleKind::Period,
            Some(period),
            grace,
            None,
            "UTC",
        )
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
    let events = scan_once(&store, Utc::now()).await.unwrap();
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
    let events = scan_once(&store, Utc::now()).await.unwrap();
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

    // First scan: transitions the check to Down and emits exactly one event.
    let events = scan_once(&store, now).await.unwrap();
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

    // Second scan with the same (or later) `now`: the check is already Down,
    // so it's excluded from list_active_checks and must not be re-emitted.
    let events = scan_once(&store, now).await.unwrap();
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
    let events = scan_once(&store, due).await.unwrap();
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

    // now == due - 1s: still not due yet, must not emit or down the check.
    let events = scan_once(&store, due - Duration::seconds(1)).await.unwrap();
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

    // build store with an overdue up check bound to a webhook channel
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

    let events = scan_once(&store, now).await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].check_id, id);
    for ev in &events {
        deliver_event(&store, ev, RetryPolicy::default(), now).await;
    }
    assert_eq!(
        store.list_recent_notifications(id, 10).await.unwrap()[0].status,
        NotifyStatus::Ok
    );
}
