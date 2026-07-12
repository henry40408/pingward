use chrono::{Duration, Utc};
use pingward::{
    db,
    models::{CheckStatus, ScheduleKind},
    scheduler::scan_once,
    store::Store,
};

async fn store_with_up_check(period: i64, grace: i64, last_ping_ago: i64) -> (Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    sqlx::query("INSERT INTO users (username,is_admin,created_at) VALUES ('u',0,datetime('now'))")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO projects (user_id,name,created_at) VALUES (1,'p',datetime('now'))")
        .execute(&pool)
        .await
        .unwrap();
    let store = Store::new(pool);
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
