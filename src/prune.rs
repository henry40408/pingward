use crate::shutdown::Shutdown;
use crate::store::Store;
use chrono::{DateTime, Duration, Utc};
use tokio::time::{Duration as TokioDuration, sleep};

/// Parse a `settings` retention value into a positive day count, or `None`
/// (retention off) when unset, blank, non-numeric, or non-positive.
fn parse_days(v: Option<String>) -> Option<i64> {
    v.and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&n| n > 0)
}

/// Resolve a retention setting into an RFC3339 cutoff: rows with
/// `created_at < cutoff` should be deleted. Returns `None` when retention is
/// off (unset/blank/non-numeric/≤0) or the day count is so large that
/// `now - days` would overflow the representable range. In the overflow case a
/// warning is logged and nothing is pruned — a fail-safe that keeps data
/// rather than panicking the prune task.
fn retention_cutoff(now: DateTime<Utc>, setting: Option<String>) -> Option<String> {
    let days = parse_days(setting)?;
    if let Some(cutoff) = Duration::try_days(days).and_then(|d| now.checked_sub_signed(d)) {
        Some(cutoff.to_rfc3339())
    } else {
        tracing::warn!("retention of {days} days is out of range; skipping prune this run");
        None
    }
}

/// Which table a prune pass targets. Ties the retention setting key to the
/// matching delete method so the two are impossible to mismatch.
#[derive(Clone, Copy)]
enum PruneTable {
    Pings,
    Notifications,
}

impl PruneTable {
    fn setting_key(self) -> &'static str {
        match self {
            PruneTable::Pings => "pings_retention_days",
            PruneTable::Notifications => "notifications_retention_days",
        }
    }
}

/// Prune one table: resolve its retention cutoff and delete rows older than it,
/// or return 0 when retention is off. Returns the number of rows deleted.
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
    }
}

/// Delete `pings` and `notifications` older than their configured retention,
/// plus any `sessions` row that has already expired. The pings/notifications
/// counts are each an independent global retention setting; a table with
/// retention off is skipped (its count is 0). Expired-session cleanup is
/// unconditional — it does not participate in retention settings, since an
/// expired session is already unusable regardless of how long it's kept
/// around. Returns `(pings_deleted, notifications_deleted, sessions_deleted)`.
/// `now` is injected for determinism.
pub async fn prune_once(store: &Store, now: DateTime<Utc>) -> Result<(u64, u64, u64), sqlx::Error> {
    let pings_deleted = prune_table(store, now, PruneTable::Pings).await?;
    let notifications_deleted = prune_table(store, now, PruneTable::Notifications).await?;
    let sessions_deleted = store.delete_expired_sessions(&now.to_rfc3339()).await?;
    Ok((pings_deleted, notifications_deleted, sessions_deleted))
}

/// Run the prune task until shutdown: prune once immediately, then every
/// `interval_secs` (bounded to >= 1s). Errors are logged, never fatal.
///
/// `shutdown` is checked at the sleep, so a delete pass already in flight
/// finishes instead of being abandoned. Returning lets `main` close the pool
/// with no query outstanding (see `shutdown::os_signal`).
pub async fn run_prune_loop(store: Store, interval_secs: u64, shutdown: Shutdown) {
    let interval = TokioDuration::from_secs(interval_secs.max(1));
    loop {
        match prune_once(&store, Utc::now()).await {
            Ok((p, n, s)) => {
                if p > 0 || n > 0 || s > 0 {
                    tracing::info!("pruned {p} pings, {n} notifications, {s} sessions");
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
        // retention off → None
        assert_eq!(retention_cutoff(now, None), None);
        assert_eq!(retention_cutoff(now, Some("0".into())), None);
        // sane value → cutoff = now - N days
        assert_eq!(
            retention_cutoff(now, Some("7".into())),
            Some((now - Duration::days(7)).to_rfc3339())
        );
        // absurd value must NOT panic — fail-safe to None (overflow branch)
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

        let (p, n, s) = prune_once(&store, now).await.unwrap();
        assert_eq!((p, n, s), (1, 1, 0));
        assert_eq!(store.list_recent_pings(cid, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn prune_heartbeat_setting_writes() {
        let (store, _cid, _chan) = store_with_check_and_channel().await;
        // Simulate one loop body's heartbeat write.
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
        assert_eq!(prune_once(&store, now).await.unwrap(), (0, 0, 0));
        // explicit 0 → off
        store
            .set_setting("pings_retention_days", "0")
            .await
            .unwrap();
        assert_eq!(prune_once(&store, now).await.unwrap(), (0, 0, 0));
        assert_eq!(store.list_recent_pings(cid, 10).await.unwrap().len(), 1);
    }

    /// The shutdown flag ends the prune loop, and it ends by *returning* — a
    /// completed `JoinHandle` (not an aborted one) is what lets `main` close
    /// the pool knowing no delete pass is still in flight.
    #[tokio::test]
    async fn run_prune_loop_returns_on_shutdown() {
        let (store, _cid, _chan) = store_with_check_and_channel().await;
        let (shutdown_tx, shutdown) = crate::shutdown::channel();
        // A one-hour interval: without the select on `shutdown`, the loop would
        // sit in `sleep` far past this test's timeout.
        let handle = tokio::spawn(run_prune_loop(store, 3600, shutdown));

        shutdown_tx.trigger();

        tokio::time::timeout(TokioDuration::from_secs(5), handle)
            .await
            .expect("run_prune_loop must return promptly after shutdown is triggered")
            .expect("run_prune_loop must return normally, not panic");
    }

    /// The other side: with the flag untouched the loop keeps running, so the
    /// test above is proving the trigger works rather than that the loop always
    /// exits.
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
                now,
            )
            .await
            .unwrap();

        let (p, n, s) = prune_once(&store, now).await.unwrap();
        assert_eq!((p, n, s), (0, 0, 1));

        let remaining = store.list_sessions_for_user(user_id, now).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "sess-valid");
    }
}
