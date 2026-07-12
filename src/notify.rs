use chrono::{DateTime, Utc};
use std::future::Future;
use std::pin::Pin;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Down,
    Up,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::Down => "down",
            EventKind::Up => "up",
        }
    }
}

impl std::str::FromStr for EventKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "down" => Ok(EventKind::Down),
            "up" => Ok(EventKind::Up),
            other => Err(format!("invalid EventKind: {other}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NotificationEvent {
    pub check_id: i64,
    pub check_name: String,
    pub event: EventKind,
    pub at: DateTime<Utc>,
    pub project_id: i64,
}

#[derive(Debug, thiserror::Error)]
#[error("notify failed: {0}")]
pub struct NotifyError(pub String);

pub trait Notifier: Send + Sync {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>>;
}

pub struct WebhookNotifier {
    url: String,
    client: reqwest::Client,
}

impl WebhookNotifier {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

impl Notifier for WebhookNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({
                "check": ev.check_name,
                "event": ev.event.as_str(),
                "at": ev.at.to_rfc3339(),
                "project_id": ev.project_id,
            });
            let resp = self
                .client
                .post(&self.url)
                .json(&body)
                .send()
                .await
                .map_err(|e| NotifyError(e.to_string()))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(NotifyError(format!("status {}", resp.status())))
            }
        })
    }
}

use crate::models::{Channel, ChannelKind, NotifyStatus};
use crate::store::Store;

#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_backoff: std::time::Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_backoff: std::time::Duration::from_millis(500),
        }
    }
}

/// Build a notifier for a channel. Only `webhook` is implemented in Plan 2;
/// Telegram/Slack/ntfy return `None` (logged) and arrive in Plan 3.
pub fn notifier_for(channel: &Channel) -> Option<Box<dyn Notifier>> {
    match channel.kind {
        ChannelKind::Webhook => {
            let url = serde_json::from_str::<serde_json::Value>(&channel.config_json)
                .ok()
                .and_then(|v| v.get("url").and_then(|u| u.as_str()).map(str::to_owned));
            match url {
                Some(u) => Some(Box::new(WebhookNotifier::new(u))),
                None => {
                    tracing::warn!(
                        channel_id = channel.id,
                        "webhook channel missing url in config_json"
                    );
                    None
                }
            }
        }
        other => {
            tracing::debug!(
                channel_id = channel.id,
                kind = other.as_str(),
                "channel kind not yet supported (Plan 3)"
            );
            None
        }
    }
}

/// Send with bounded exponential-backoff retry. Returns the last error if all
/// attempts fail.
pub async fn send_with_retry(
    n: &dyn Notifier,
    ev: &NotificationEvent,
    policy: RetryPolicy,
) -> Result<(), NotifyError> {
    let mut last = NotifyError("no attempts".into());
    for attempt in 0..policy.max_attempts.max(1) {
        match n.send(ev).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = e;
                if attempt + 1 < policy.max_attempts {
                    let backoff = policy.base_backoff * 2u32.saturating_pow(attempt);
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }
    Err(last)
}

/// Resolve the check's bound channels, deliver to each with retry, and record
/// every outcome in `notifications`. Delivery failures are recorded, never
/// propagated (spec §6: a failing channel must not affect state).
pub async fn deliver_event(
    store: &Store,
    ev: &NotificationEvent,
    policy: RetryPolicy,
    now: DateTime<Utc>,
) {
    let channels = match store.channels_for_check(ev.check_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(check_id = ev.check_id, "failed to load channels: {e}");
            return;
        }
    };
    if channels.is_empty() {
        tracing::debug!(
            check = %ev.check_name,
            event = ev.event.as_str(),
            "no channels bound; nothing to deliver"
        );
        return;
    }
    for channel in &channels {
        let Some(notifier) = notifier_for(channel) else {
            continue;
        };
        let (status, error) = match send_with_retry(notifier.as_ref(), ev, policy).await {
            Ok(()) => (NotifyStatus::Ok, None),
            Err(e) => (NotifyStatus::Error, Some(e.to_string())),
        };
        if let Err(e) = store
            .record_notification(
                ev.check_id,
                channel.id,
                ev.event,
                status,
                error.as_deref(),
                now,
            )
            .await
        {
            tracing::error!(
                check_id = ev.check_id,
                channel_id = channel.id,
                "failed to record notification: {e}"
            );
        }
    }
}

pub async fn dispatch(
    notifiers: &[Box<dyn Notifier>],
    ev: &NotificationEvent,
) -> Vec<Result<(), NotifyError>> {
    let mut out = Vec::with_capacity(notifiers.len());
    for n in notifiers {
        out.push(n.send(ev).await);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn webhook_posts_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let n = WebhookNotifier::new(format!("{}/hook", server.uri()));
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
        };
        n.send(&ev).await.unwrap();
        // wiremock verifies expect(1) on drop
    }

    /// A hung endpoint must not block delivery forever: the client's 10s
    /// timeout should fire and `send` should return `Err` well before the
    /// mock's 30s delay elapses. This test adds ~10s of real wall-clock time
    /// (reqwest's timer is real; tokio's paused clock does not apply to it).
    #[tokio::test]
    async fn webhook_send_times_out_on_hung_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/hook"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30)))
            .mount(&server)
            .await;

        let n = WebhookNotifier::new(format!("{}/hook", server.uri()));
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
        };

        let start = std::time::Instant::now();
        let result = n.send(&ev).await;
        let elapsed = start.elapsed();

        assert!(result.is_err(), "expected timeout to produce an error");
        assert!(
            elapsed < std::time::Duration::from_secs(20),
            "send took {:?}, expected the 10s client timeout to fire well before the 30s mock delay",
            elapsed
        );
    }

    use crate::db;
    use crate::models::ChannelKind;
    use crate::store::Store;

    async fn store_with_check_and_channel(url: &str) -> (Store, i64) {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (username,is_admin,created_at) VALUES ('u',0,datetime('now'))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO projects (user_id,name,created_at) VALUES (1,'p',datetime('now'))",
        )
        .execute(&pool)
        .await
        .unwrap();
        let store = Store::new(pool);
        let now = Utc::now();
        let cid = store
            .create_channel(
                1,
                ChannelKind::Webhook,
                "hook",
                &format!("{{\"url\":\"{url}\"}}"),
                now,
            )
            .await
            .unwrap();
        let chk = store
            .create_check(
                1,
                "job",
                "u1",
                crate::models::ScheduleKind::Period,
                Some(60),
                30,
                None,
                "UTC",
            )
            .await
            .unwrap();
        store.bind_channel(chk, cid).await.unwrap();
        (store, chk)
    }

    #[tokio::test]
    async fn deliver_event_posts_and_records_ok() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let (store, chk) = store_with_check_and_channel(&server.uri()).await;
        let ev = NotificationEvent {
            check_id: chk,
            check_name: "job".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
        };
        deliver_event(&store, &ev, RetryPolicy::default(), Utc::now()).await;

        let recs = store.list_recent_notifications(chk, 10).await.unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].status, crate::models::NotifyStatus::Ok);
    }

    #[tokio::test]
    async fn deliver_event_records_error_when_channel_fails() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let (store, chk) = store_with_check_and_channel(&server.uri()).await;
        let ev = NotificationEvent {
            check_id: chk,
            check_name: "job".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
        };
        // fast policy so the test does not sleep for seconds
        let policy = RetryPolicy {
            max_attempts: 2,
            base_backoff: std::time::Duration::from_millis(1),
        };
        deliver_event(&store, &ev, policy, Utc::now()).await;

        let recs = store.list_recent_notifications(chk, 10).await.unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].status, crate::models::NotifyStatus::Error);
        assert!(recs[0].error.is_some());
    }
}
