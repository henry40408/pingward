use pingward::{
    db,
    models::{ChannelKind, ScheduleKind},
    store::{NewCheck, Store},
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
        .create_check(&NewCheck {
            project_id: pid,
            name: "job",
            ping_uuid: "uuid-1",
            kind: ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
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

    // batched recent-pings query (the dashboard's N+1 avoidance): the
    // ROW_NUMBER() window must behave on PostgreSQL exactly as on SQLite —
    // per-check limit honored, grouped by check_id, and matching the per-check
    // query. cid already has one ping; give cid2 three.
    let cid2 = store
        .create_check(&NewCheck {
            project_id: pid,
            name: "job2",
            ping_uuid: "uuid-2",
            kind: ScheduleKind::Period,
            period_secs: Some(60),
            grace_secs: 30,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    for i in 0..3 {
        store
            .insert_ping(
                cid2,
                pingward::models::PingKind::Success,
                None,
                "p",
                None,
                now + chrono::Duration::seconds(i),
            )
            .await
            .unwrap();
    }
    let batch = store
        .list_recent_pings_for_checks(&[cid, cid2], 2)
        .await
        .unwrap();
    assert_eq!(batch.get(&cid).unwrap().len(), 1);
    assert_eq!(
        batch.get(&cid2).unwrap().len(),
        2,
        "per-check limit honored"
    );
    let per_check: Vec<i64> = store
        .list_recent_pings(cid2, 2)
        .await
        .unwrap()
        .iter()
        .map(|p| p.id)
        .collect();
    let batched: Vec<i64> = batch.get(&cid2).unwrap().iter().map(|p| p.id).collect();
    assert_eq!(batched, per_check, "batch order matches per-check query");

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
        .create_session("sess-active", uid, "csrf-active", future_expiry)
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
        .create_session("sess-expired", uid, "csrf-expired", past_expiry)
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
            &pingward::store::UpdateCheck {
                name: "job",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                cron_expr: None,
                timezone: "UTC",
                scan_interval_secs: None,
                max_runtime_secs: None,
                nag_interval_secs: Some(60),
            },
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
    // Exactly the one 30-day-old ping and one 30-day-old notification are
    // pruned; every other row in this test was inserted at `now`. The exact
    // counts guard against an over-deleting regression that `>= 1` would miss.
    assert_eq!(
        (pd, nd),
        (1, 1),
        "expected exactly the old ping+notification pruned, got ({pd},{nd})"
    );
    // direct delete methods also work with an explicit cutoff: a far-past
    // cutoff matches nothing in either table (every remaining row is recent).
    let far = (now - chrono::Duration::days(3650)).to_rfc3339();
    assert_eq!(store.delete_pings_before(&far).await.unwrap(), 0);
    assert_eq!(store.delete_notifications_before(&far).await.unwrap(), 0);

    // api keys: insert/list/validate/owner-scoped-delete/expiry, all on PG.
    let (_full, prefix, hash) = pingward::apikey::generate_api_key();
    let kid = store
        .insert_api_key(uid, "ci", &hash, &prefix, None, now)
        .await
        .unwrap();
    assert_eq!(store.list_api_keys_for_user(uid).await.unwrap().len(), 1);
    assert_eq!(store.validate_api_key(&hash, now).await.unwrap(), Some(uid));
    // Owner-scoped delete: a non-owner id can't remove it; the owner can.
    let stranger = store
        .create_user("stranger", Some("phc"), false, now)
        .await
        .unwrap();
    assert!(!store.delete_api_key(kid, stranger).await.unwrap());
    assert!(store.delete_api_key(kid, uid).await.unwrap());
    assert!(store.list_api_keys_for_user(uid).await.unwrap().is_empty());
    // Expired keys are rejected.
    let (_f2, p2, h2) = pingward::apikey::generate_api_key();
    store
        .insert_api_key(
            uid,
            "old",
            &h2,
            &p2,
            Some(now - chrono::Duration::hours(1)),
            now,
        )
        .await
        .unwrap();
    assert_eq!(store.validate_api_key(&h2, now).await.unwrap(), None);
    // A live key remains, to prove the user-delete cascade removes it below.
    let (_f3, p3, h3) = pingward::apikey::generate_api_key();
    store
        .insert_api_key(uid, "live", &h3, &p3, None, now)
        .await
        .unwrap();

    // cascade delete: removing the user removes project → checks → channels →
    // pings, and the user's api keys.
    store.delete_user(uid).await.unwrap();
    assert!(store.list_projects_for_user(uid).await.unwrap().is_empty());
    assert!(store.find_check(cid).await.unwrap().is_none());
    assert_eq!(store.validate_api_key(&h3, now).await.unwrap(), None);
}
