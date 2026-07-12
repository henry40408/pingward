use crate::store::Store;
use chrono::{DateTime, Duration, Utc};
use tokio::time::{sleep, Duration as TokioDuration};

/// Parse a `settings` retention value into a positive day count, or `None`
/// (retention off) when unset, blank, non-numeric, or non-positive.
fn parse_days(v: Option<String>) -> Option<i64> {
    v.and_then(|s| s.trim().parse::<i64>().ok())
        .filter(|&n| n > 0)
}

/// Delete `pings` and `notifications` older than their configured retention.
/// Each table's retention is an independent global setting; a table with
/// retention off is skipped (its count is 0). Returns
/// `(pings_deleted, notifications_deleted)`. `now` is injected for determinism.
pub async fn prune_once(store: &Store, now: DateTime<Utc>) -> Result<(u64, u64), sqlx::Error> {
    let pings_deleted = match parse_days(store.get_setting("pings_retention_days").await?) {
        Some(days) => {
            let cutoff = (now - Duration::days(days)).to_rfc3339();
            store.delete_pings_before(&cutoff).await?
        }
        None => 0,
    };
    let notifications_deleted =
        match parse_days(store.get_setting("notifications_retention_days").await?) {
            Some(days) => {
                let cutoff = (now - Duration::days(days)).to_rfc3339();
                store.delete_notifications_before(&cutoff).await?
            }
            None => 0,
        };
    Ok((pings_deleted, notifications_deleted))
}

/// Run the prune task forever: prune once immediately, then every
/// `interval_secs` (bounded to >= 1s). Errors are logged, never fatal.
pub async fn run_prune_loop(store: Store, interval_secs: u64) {
    let interval = TokioDuration::from_secs(interval_secs.max(1));
    loop {
        match prune_once(&store, Utc::now()).await {
            Ok((p, n)) => {
                if p > 0 || n > 0 {
                    tracing::info!("pruned {p} pings, {n} notifications");
                }
            }
            Err(e) => tracing::error!("prune_once failed: {e}"),
        }
        sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::models::{ChannelKind, NotifyStatus, PingKind, ScheduleKind};
    use crate::notify::EventKind;
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
            .create_project(1, "p", None, None, Utc::now())
            .await
            .unwrap();
        let cid = store
            .create_check(
                1,
                "c",
                "uu",
                ScheduleKind::Period,
                Some(60),
                30,
                None,
                "UTC",
            )
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

        let (p, n) = prune_once(&store, now).await.unwrap();
        assert_eq!((p, n), (1, 1));
        assert_eq!(store.list_recent_pings(cid, 10).await.unwrap().len(), 1);
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
        assert_eq!(prune_once(&store, now).await.unwrap(), (0, 0));
        // explicit 0 → off
        store
            .set_setting("pings_retention_days", "0")
            .await
            .unwrap();
        assert_eq!(prune_once(&store, now).await.unwrap(), (0, 0));
        assert_eq!(store.list_recent_pings(cid, 10).await.unwrap().len(), 1);
    }
}
