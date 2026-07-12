use pingward::{
    db,
    models::{ChannelKind, ScheduleKind},
    store::Store,
};

fn pg_url() -> Option<String> {
    std::env::var("TEST_DATABASE_URL")
        .ok()
        .filter(|u| u.starts_with("postgres"))
}

async fn fresh_pg_store(url: &str) -> Store {
    let pool = db::connect(url).await.expect("connect postgres");
    // Reset to a clean schema so migrations apply idempotently across runs.
    sqlx::query("DROP SCHEMA public CASCADE")
        .execute(&pool)
        .await
        .ok();
    sqlx::query("CREATE SCHEMA public")
        .execute(&pool)
        .await
        .expect("recreate schema");
    db::migrate(&pool, url).await.expect("migrate postgres");
    Store::new(pool)
}

#[tokio::test]
async fn postgres_full_round_trip() {
    let Some(url) = pg_url() else {
        eprintln!("TEST_DATABASE_URL unset — skipping postgres_full_round_trip");
        return;
    };
    let store = fresh_pg_store(&url).await;
    let now = chrono::Utc::now();

    // users
    let uid = store
        .create_user("alice", Some("phc"), true, now)
        .await
        .unwrap();
    assert!(
        store
            .find_user_by_username("alice")
            .await
            .unwrap()
            .unwrap()
            .is_admin
    );

    // projects
    let pid = store
        .create_project(uid, "web", Some(45), None, now)
        .await
        .unwrap();
    assert_eq!(store.list_projects_for_user(uid).await.unwrap().len(), 1);

    // checks
    let cid = store
        .create_check(
            pid,
            "job",
            "uuid-1",
            ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();
    assert!(store.find_check(cid).await.unwrap().is_some());
    assert_eq!(store.list_checks_for_project(pid).await.unwrap().len(), 1);
    assert_eq!(store.list_active_checks().await.unwrap().len(), 1);

    // channels + binding
    let chid = store
        .create_channel(
            pid,
            ChannelKind::Webhook,
            "hook",
            "{\"url\":\"http://x\"}",
            now,
        )
        .await
        .unwrap();
    store.bind_channel(cid, chid).await.unwrap();
    assert_eq!(store.bound_channel_ids(cid).await.unwrap(), vec![chid]);
    assert_eq!(store.channels_for_check(cid).await.unwrap().len(), 1);

    // pings + status transition
    store
        .insert_ping(
            cid,
            pingward::models::PingKind::Success,
            None,
            "ok",
            Some("127.0.0.1"),
            now,
        )
        .await
        .unwrap();
    store
        .mark_ping(
            cid,
            pingward::models::CheckStatus::Up,
            Some(now),
            None,
            None,
        )
        .await
        .unwrap();

    // notifications
    store
        .record_notification(
            cid,
            chid,
            pingward::notify::EventKind::Down,
            pingward::models::NotifyStatus::Ok,
            None,
            now,
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .list_recent_notifications(cid, 10)
            .await
            .unwrap()
            .len(),
        1
    );

    // settings
    store.set_setting("scan_interval", "45").await.unwrap();
    assert_eq!(
        store.get_setting("scan_interval").await.unwrap().as_deref(),
        Some("45")
    );

    // sessions
    let future_expiry = now + chrono::Duration::hours(1);
    store
        .create_session("sess-active", uid, future_expiry)
        .await
        .unwrap();
    assert_eq!(
        store
            .find_session_user("sess-active", now)
            .await
            .unwrap()
            .map(|u| u.username),
        Some("alice".to_string())
    );

    let past_expiry = now - chrono::Duration::hours(1);
    store
        .create_session("sess-expired", uid, past_expiry)
        .await
        .unwrap();
    assert!(
        store
            .find_session_user("sess-expired", now)
            .await
            .unwrap()
            .is_none(),
        "expired session must not resolve to a user"
    );

    store.delete_session("sess-active").await.unwrap();
    assert!(
        store
            .find_session_user("sess-active", now)
            .await
            .unwrap()
            .is_none(),
        "deleted session must not resolve to a user"
    );

    // nag: configure a per-check interval, down the check, stamp a baseline,
    // and confirm the reminder scan and acknowledge/clear cycle work on PG.
    store
        .update_check_schedule(
            cid,
            "job",
            ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
            None,
            None,
            Some(60),
        )
        .await
        .unwrap();
    store
        .set_status(cid, pingward::models::CheckStatus::Down)
        .await
        .unwrap();
    let t0 = now;
    store.begin_down_alert(cid, t0).await.unwrap();
    let due = t0 + chrono::Duration::seconds(90);
    let evs = pingward::scheduler::nag_once(&store, due).await.unwrap();
    assert!(evs
        .iter()
        .any(|e| e.check_id == cid && e.event == pingward::notify::EventKind::Reminder));
    store.acknowledge(cid).await.unwrap();
    assert!(store.find_check(cid).await.unwrap().unwrap().acknowledged);
    // acknowledged → no further reminders
    assert!(
        pingward::scheduler::nag_once(&store, due + chrono::Duration::seconds(300))
            .await
            .unwrap()
            .into_iter()
            .all(|e| e.check_id != cid)
    );
    store.clear_nag(cid).await.unwrap();
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().last_alert_at,
        None
    );

    // retention/pruning: an old ping + old notification are deleted by prune_once
    // when retention is configured; a far-future cutoff via a large retention
    // keeps recent rows.
    let old = now - chrono::Duration::days(30);
    store
        .insert_ping(
            cid,
            pingward::models::PingKind::Success,
            None,
            "",
            None,
            old,
        )
        .await
        .unwrap();
    store
        .record_notification(
            cid,
            chid,
            pingward::notify::EventKind::Down,
            pingward::models::NotifyStatus::Ok,
            None,
            old,
        )
        .await
        .unwrap();
    store
        .set_setting("pings_retention_days", "7")
        .await
        .unwrap();
    store
        .set_setting("notifications_retention_days", "7")
        .await
        .unwrap();
    let (pd, nd) = pingward::prune::prune_once(&store, now).await.unwrap();
    assert!(
        pd >= 1 && nd >= 1,
        "expected old ping+notification pruned, got ({pd},{nd})"
    );
    // direct delete method also works with an explicit cutoff
    let far = (now - chrono::Duration::days(3650)).to_rfc3339();
    assert_eq!(store.delete_pings_before(&far).await.unwrap(), 0);

    // cascade delete: removing the user removes project → checks → channels → pings
    store.delete_user(uid).await.unwrap();
    assert!(store.list_projects_for_user(uid).await.unwrap().is_empty());
    assert!(store.find_check(cid).await.unwrap().is_none());
}
