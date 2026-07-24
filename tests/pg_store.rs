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
        .create_project(uid, "web", "", Some(45), None, now)
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

    // batched checks-per-project query (the other half of the dashboard's N+1
    // avoidance): the generated `IN ($1,…,$N)` list must bind and group on
    // PostgreSQL exactly as on SQLite. `pid2` is deliberately empty, so the
    // "absent, not empty vector" contract is exercised here too.
    let pid2 = store
        .create_project(uid, "proj2", "", None, None, now)
        .await
        .unwrap();
    let by_project = store.list_checks_for_projects(&[pid, pid2]).await.unwrap();
    let batched: Vec<i64> = by_project.get(&pid).unwrap().iter().map(|c| c.id).collect();
    let per_project: Vec<i64> = store
        .list_checks_for_project(pid)
        .await
        .unwrap()
        .iter()
        .map(|c| c.id)
        .collect();
    assert_eq!(batched, per_project, "batch matches per-project query");
    assert!(
        !by_project.contains_key(&pid2),
        "a project with no checks must be absent from the map"
    );

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
        .create_session(
            "sess-active",
            uid,
            future_expiry,
            Some("curl/8.0"),
            Some("127.0.0.1"),
            false,
            now,
        )
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
        .create_session("sess-expired", uid, past_expiry, None, None, false, now)
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

    // list_sessions_for_user only surfaces the still-valid session, with the
    // metadata stamped at creation and `last_seen_at` stamped by the
    // `find_session_user` lookup above.
    let sessions = store.list_sessions_for_user(uid, now).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "sess-active");
    assert_eq!(sessions[0].user_agent.as_deref(), Some("curl/8.0"));
    assert_eq!(sessions[0].ip.as_deref(), Some("127.0.0.1"));
    assert_eq!(sessions[0].last_seen_at, Some(now));

    // A second session, then "revoke others" keeps only it.
    store
        .create_session(
            "sess-second",
            uid,
            future_expiry,
            None,
            None,
            false,
            now + chrono::Duration::seconds(1),
        )
        .await
        .unwrap();
    // Removes both "sess-active" and the already-expired "sess-expired" —
    // "revoke others" is not conditioned on expiry, only on not being `keep_id`.
    let removed = store
        .delete_other_sessions_for_user(uid, "sess-second")
        .await
        .unwrap();
    assert_eq!(removed, 2);
    let sessions = store.list_sessions_for_user(uid, now).await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, "sess-second");

    // Owner-scoped delete: another user's id is a no-op, the owner's id works.
    assert!(
        !store
            .delete_session_owned("sess-second", uid + 1)
            .await
            .unwrap()
    );
    assert_eq!(
        store.list_sessions_for_user(uid, now).await.unwrap().len(),
        1
    );
    assert!(
        store
            .delete_session_owned("sess-second", uid)
            .await
            .unwrap()
    );
    assert!(
        store
            .list_sessions_for_user(uid, now)
            .await
            .unwrap()
            .is_empty()
    );

    // Plain `delete_session` (used by logout) still works unscoped.
    store
        .create_session("sess-active", uid, future_expiry, None, None, false, now)
        .await
        .unwrap();
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
                description: "",
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
    assert!(
        evs.iter()
            .any(|e| e.check_id == cid && e.event == pingward::notify::EventKind::Reminder)
    );
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
    // An expired session must also be pruned, unconditionally, alongside the
    // retention-driven pings and notifications.
    store
        .create_session(
            "sess-prune-expired",
            uid,
            now - chrono::Duration::hours(1),
            None,
            None,
            false,
            now - chrono::Duration::hours(2),
        )
        .await
        .unwrap();
    store
        .create_session(
            "sess-prune-valid",
            uid,
            now + chrono::Duration::hours(1),
            None,
            None,
            false,
            now,
        )
        .await
        .unwrap();

    let (pd, nd, sd) = pingward::prune::prune_once(&store, now).await.unwrap();
    // Exactly the one 30-day-old ping, one 30-day-old notification, and one
    // expired session are pruned; every other row in this test was inserted
    // at `now`/future. The exact counts guard against an over-deleting
    // regression that `>= 1` would miss.
    assert_eq!(
        (pd, nd, sd),
        (1, 1, 1),
        "expected exactly the old ping+notification+expired session pruned, got ({pd},{nd},{sd})"
    );
    let remaining_sessions = store.list_sessions_for_user(uid, now).await.unwrap();
    assert_eq!(remaining_sessions.len(), 1);
    assert_eq!(remaining_sessions[0].id, "sess-prune-valid");
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
