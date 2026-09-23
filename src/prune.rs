use crate::shutdown::Shutdown;
use crate::store::Store;
use chrono::{DateTime, Duration, Utc};
use tokio::time::{Duration as TokioDuration, sleep};

/// `None` (retention off) when unset, blank, non-numeric, or non-positive.
fn parse_days(v: Option<String>) -> Option<i64> {
    v.and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&n| n > 0)
}

/// RFC3339 cutoff for `created_at < cutoff`. `None` when retention is off or
/// `now - days` overflows (warns rather than panicking the task).
fn retention_cutoff(now: DateTime<Utc>, setting: Option<String>) -> Option<String> {
    let days = parse_days(setting)?;
    if let Some(cutoff) = Duration::try_days(days).and_then(|d| now.checked_sub_signed(d)) {
        Some(cutoff.to_rfc3339())
    } else {
        tracing::warn!("retention of {days} days is out of range; skipping prune this run");
        None
    }
}

/// Ties each retention setting key to its delete method.
#[derive(Clone, Copy)]
enum PruneTable {
    Pings,
    Notifications,
    Audit,
}

impl PruneTable {
    fn setting_key(self) -> &'static str {
        match self {
            PruneTable::Pings => "pings_retention_days",
            PruneTable::Notifications => "notifications_retention_days",
            PruneTable::Audit => "audit_retention_days",
        }
    }
}

/// Rows deleted; 0 when retention is off.
async fn prune_table(
    store: &Store,
    now: DateTime<Utc>,
    table: PruneTable,
) -> Result<u64, sqlx::Error> {
    let Some(cutoff) = retention_cutoff(now, store.get_setting(table.setting_key()).await?) else {
        return Ok(0);
    };
    match table {
        PruneTable::Pings => store.delete_pings_before(&cutoff).await,
        PruneTable::Notifications => store.delete_notifications_before(&cutoff).await,
        PruneTable::Audit => store.delete_audit_before(&cutoff).await,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PruneCounts {
    pub pings: u64,
    pub notifications: u64,
    pub audit: u64,
    pub sessions: u64,
}

impl PruneCounts {
    fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// Delete `pings`, `notifications` and `audit_log` rows past their retention,
/// plus expired `sessions` (unconditionally). Every retention defaults to off,
/// so an upgrade never starts deleting audit records.
pub async fn prune_once(store: &Store, now: DateTime<Utc>) -> Result<PruneCounts, sqlx::Error> {
    Ok(PruneCounts {
        pings: prune_table(store, now, PruneTable::Pings).await?,
        notifications: prune_table(store, now, PruneTable::Notifications).await?,
        audit: prune_table(store, now, PruneTable::Audit).await?,
        sessions: store.delete_expired_sessions(now).await?,
    })
}

/// Prune now, then every `interval_secs` (min 1s). `shutdown` is checked only at
/// the sleep, so an in-flight pass finishes before the pool closes.
pub async fn run_prune_loop(store: Store, interval_secs: u64, shutdown: Shutdown) {
    let interval = TokioDuration::from_secs(interval_secs.max(1));
    loop {
        match prune_once(&store, Utc::now()).await {
            Ok(c) => {
                if !c.is_empty() {
                    tracing::info!(
                        "pruned {} pings, {} notifications, {} audit entries, {} sessions",
                        c.pings,
                        c.notifications,
                        c.audit,
                        c.sessions
                    );
                }
                if c.sessions > 0 {
                    // Aggregate: `delete_expired_sessions` returns only a count.
                    tracing::info!(
                        target: "pingward::session",
                        reason = "expired",
                        count = c.sessions,
                        "session.destroyed"
                    );
                }
            }
            Err(e) => tracing::error!("prune_once failed: {e}"),
        }
        let _ = store
            .set_setting("last_prune_at", &Utc::now().to_rfc3339())
            .await;
        tokio::select! {
            () = sleep(interval) => {}
            () = shutdown.wait() => {
                tracing::info!("prune loop stopping");
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::models::{ChannelKind, NotifyStatus, PingKind, ScheduleKind};
    use crate::notify::EventKind;
    use crate::store::NewCheck;
    use chrono::TimeZone;

    async fn store_with_check_and_channel() -> (Store, i64, i64) {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool, "sqlite::memory:").await.unwrap();
        let store = Store::new(pool);
        store
            .create_user("u", None, false, Utc::now())
            .await
            .unwrap();
        store
            .create_project(1, "p", "", None, None, Utc::now())
            .await
            .unwrap();
        let cid = store
            .create_check(&NewCheck {
                project_id: 1,
                name: "c",
                ping_uuid: "uu",
                kind: ScheduleKind::Period,
                period_secs: Some(60),
                grace_secs: 30,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
        let chan = store
            .create_channel(
                1,
                ChannelKind::Webhook,
                "h",
                "{\"url\":\"http://x\"}",
                Utc::now(),
            )
            .await
            .unwrap();
        (store, cid, chan)
    }

    #[test]
    fn parse_days_off_and_positive_cases() {
        assert_eq!(parse_days(None), None);
        assert_eq!(parse_days(Some(String::new())), None);
        assert_eq!(parse_days(Some("   ".into())), None);
        assert_eq!(parse_days(Some("abc".into())), None);
        assert_eq!(parse_days(Some("0".into())), None);
        assert_eq!(parse_days(Some("-5".into())), None);
        assert_eq!(parse_days(Some("7".into())), Some(7));
        assert_eq!(parse_days(Some("  30 ".into())), Some(30));
    }

    #[test]
    fn retention_cutoff_off_overflow_and_valid() {
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        assert_eq!(retention_cutoff(now, None), None);
        assert_eq!(retention_cutoff(now, Some("0".into())), None);
        assert_eq!(
            retention_cutoff(now, Some("7".into())),
            Some((now - Duration::days(7)).to_rfc3339())
        );
        // An absurd value must not panic; the overflow branch fails safe.
        assert_eq!(
            retention_cutoff(now, Some("999999999999999999".into())),
            None
        );
    }

    #[tokio::test]
    async fn prune_once_deletes_old_when_retention_set() {
        let (store, cid, chan) = store_with_check_and_channel().await;
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        let old = now - Duration::days(10);
        let recent = now - Duration::days(1);
        store
            .insert_ping(cid, PingKind::Success, None, "", None, old)
            .await
            .unwrap();
        store
            .insert_ping(cid, PingKind::Success, None, "", None, recent)
            .await
            .unwrap();
        store
            .record_notification(cid, chan, EventKind::Down, NotifyStatus::Ok, None, old)
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

        let c = prune_once(&store, now).await.unwrap();
        assert_eq!(
            c,
            PruneCounts {
                pings: 1,
                notifications: 1,
                ..Default::default()
            }
        );
        assert_eq!(store.list_recent_pings(cid, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn prune_once_audit_retention_is_off_by_default_and_deletes_when_set() {
        let (store, _cid, _chan) = store_with_check_and_channel().await;
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        let uid = store
            .create_user("adm", Some("phc"), true, now)
            .await
            .unwrap();
        for (n, at) in [("old", now - Duration::days(100)), ("recent", now)] {
            store
                .record_audit(
                    &crate::store::NewAudit {
                        actor_user_id: uid,
                        actor_username: "adm",
                        action: "admin.access",
                        detail: Some(n),
                        ..Default::default()
                    },
                    at,
                )
                .await
                .unwrap();
        }

        // Unset → off.
        assert_eq!(prune_once(&store, now).await.unwrap().audit, 0);
        assert_eq!(store.list_audit(10).await.unwrap().len(), 2);

        // Explicit 0 → off.
        store
            .set_setting("audit_retention_days", "0")
            .await
            .unwrap();
        assert_eq!(prune_once(&store, now).await.unwrap().audit, 0);
        assert_eq!(store.list_audit(10).await.unwrap().len(), 2);

        store
            .set_setting("audit_retention_days", "30")
            .await
            .unwrap();
        assert_eq!(prune_once(&store, now).await.unwrap().audit, 1);
        let left = store.list_audit(10).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].detail.as_deref(), Some("recent"));
    }

    #[tokio::test]
    async fn prune_heartbeat_setting_writes() {
        let (store, _cid, _chan) = store_with_check_and_channel().await;
        store
            .set_setting("last_prune_at", &Utc::now().to_rfc3339())
            .await
            .unwrap();
        assert!(store.get_setting("last_prune_at").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn prune_once_off_when_unset_or_zero() {
        let (store, cid, _chan) = store_with_check_and_channel().await;
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        store
            .insert_ping(
                cid,
                PingKind::Success,
                None,
                "",
                None,
                now - Duration::days(100),
            )
            .await
            .unwrap();

        // unset → off
        assert!(prune_once(&store, now).await.unwrap().is_empty());
        // explicit 0 → off
        store
            .set_setting("pings_retention_days", "0")
            .await
            .unwrap();
        assert!(prune_once(&store, now).await.unwrap().is_empty());
        assert_eq!(store.list_recent_pings(cid, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn run_prune_loop_returns_on_shutdown() {
        let (store, _cid, _chan) = store_with_check_and_channel().await;
        let (shutdown_tx, shutdown) = crate::shutdown::channel();
        // One hour: only the shutdown select can end the sleep in time.
        let handle = tokio::spawn(run_prune_loop(store, 3600, shutdown));

        shutdown_tx.trigger();

        tokio::time::timeout(TokioDuration::from_secs(5), handle)
            .await
            .expect("run_prune_loop must return promptly after shutdown is triggered")
            .expect("run_prune_loop must return normally, not panic");
    }

    /// Control for the test above.
    #[tokio::test]
    async fn run_prune_loop_keeps_running_without_shutdown() {
        let (store, _cid, _chan) = store_with_check_and_channel().await;
        let (_shutdown_tx, shutdown) = crate::shutdown::channel();
        let handle = tokio::spawn(run_prune_loop(store, 3600, shutdown));

        assert!(
            tokio::time::timeout(TokioDuration::from_millis(300), handle)
                .await
                .is_err(),
            "run_prune_loop must not exit while the shutdown flag is unset"
        );
    }

    #[tokio::test]
    async fn prune_once_deletes_expired_sessions() {
        let (store, _cid, _chan) = store_with_check_and_channel().await;
        let now = Utc.with_ymd_and_hms(2026, 7, 13, 12, 0, 0).unwrap();
        let user_id = store.find_user_by_username("u").await.unwrap().unwrap().id;

        store
            .create_session(
                "sess-expired",
                user_id,
                now - Duration::hours(1),
                None,
                None,
                false,
                now - Duration::hours(2),
            )
            .await
            .unwrap();
        store
            .create_session(
                "sess-valid",
                user_id,
                now + Duration::hours(1),
                None,
                None,
                false,
                now,
            )
            .await
            .unwrap();

        let c = prune_once(&store, now).await.unwrap();
        assert_eq!(
            c,
            PruneCounts {
                sessions: 1,
                ..Default::default()
            }
        );

        let remaining = store.list_sessions_for_user(user_id, now).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "sess-valid");
    }
}
