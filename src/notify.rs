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

/// Shared reqwest client: a 10s request timeout keeps a hung endpoint from
/// blocking delivery forever. Falls back to a default client if the builder
/// fails (it never does with these options, but we avoid unwrap-panics).
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// One-line human summary of a state transition, reused by text-oriented
/// channels (Telegram, Slack, and the ntfy body).
fn event_text(ev: &NotificationEvent) -> String {
    let (emoji, word) = match ev.event {
        EventKind::Down => ("\u{1F534}", "DOWN"), // red circle
        EventKind::Up => ("\u{1F7E2}", "UP"),     // green circle
    };
    format!(
        "{emoji} {name} is {word} (as of {at})",
        name = ev.check_name,
        at = ev.at.to_rfc3339()
    )
}

/// Short title for channels with a separate title field (ntfy).
fn event_title(ev: &NotificationEvent) -> String {
    format!("pingward: {} {}", ev.check_name, ev.event.as_str())
}

pub struct WebhookNotifier {
    url: String,
    client: reqwest::Client,
}

impl WebhookNotifier {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: http_client(),
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

/// Telegram Bot API. `POST {base_url}/bot{token}/sendMessage` with a JSON
/// `{chat_id, text}` body. `base_url` is injectable so tests can point at a
/// mock server; production uses `https://api.telegram.org`.
pub struct TelegramNotifier {
    token: String,
    chat_id: String,
    base_url: String,
    client: reqwest::Client,
}

impl TelegramNotifier {
    pub fn new(token: String, chat_id: String) -> Self {
        Self::with_base_url(token, chat_id, "https://api.telegram.org".to_string())
    }

    pub fn with_base_url(token: String, chat_id: String, base_url: String) -> Self {
        Self {
            token,
            chat_id,
            base_url: base_url.trim_end_matches('/').to_string(),
            client: http_client(),
        }
    }
}

impl Notifier for TelegramNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/bot{}/sendMessage", self.base_url, self.token);
            let body = serde_json::json!({
                "chat_id": self.chat_id,
                "text": event_text(ev),
            });
            let resp = self
                .client
                .post(&url)
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

/// Slack incoming webhook: `POST {url}` with a JSON `{text}` body.
pub struct SlackNotifier {
    url: String,
    client: reqwest::Client,
}

impl SlackNotifier {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: http_client(),
        }
    }
}

impl Notifier for SlackNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({ "text": event_text(ev) });
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

/// ntfy publish: `POST {base_url}/{topic}` with the message as the body and
/// `Title`/`Priority`/`Tags` headers. An optional bearer token authenticates
/// against protected topics / self-hosted servers.
pub struct NtfyNotifier {
    base_url: String,
    topic: String,
    token: Option<String>,
    client: reqwest::Client,
}

impl NtfyNotifier {
    pub fn new(base_url: String, topic: String, token: Option<String>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            topic,
            token: token.filter(|t| !t.is_empty()),
            client: http_client(),
        }
    }
}

impl Notifier for NtfyNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/{}", self.base_url, self.topic);
            let (priority, tags) = match ev.event {
                EventKind::Down => ("high", "red_circle"),
                EventKind::Up => ("default", "green_circle"),
            };
            let mut req = self
                .client
                .post(&url)
                .header("Title", event_title(ev))
                .header("Priority", priority)
                .header("Tags", tags)
                .body(event_text(ev));
            if let Some(t) = &self.token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await.map_err(|e| NotifyError(e.to_string()))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(NotifyError(format!("status {}", resp.status())))
            }
        })
    }
}

/// Pushover: `POST {base_url}/1/messages.json` with a form body carrying the
/// app `token`, the recipient `user` key, and the `message`. `base_url` is
/// injectable so tests can point at a mock; production uses
/// `https://api.pushover.net`.
pub struct PushoverNotifier {
    token: String,
    user: String,
    base_url: String,
    client: reqwest::Client,
}

impl PushoverNotifier {
    pub fn new(token: String, user: String) -> Self {
        Self::with_base_url(token, user, "https://api.pushover.net".to_string())
    }

    pub fn with_base_url(token: String, user: String, base_url: String) -> Self {
        Self {
            token,
            user,
            base_url: base_url.trim_end_matches('/').to_string(),
            client: http_client(),
        }
    }
}

impl Notifier for PushoverNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/1/messages.json", self.base_url);
            let priority = match ev.event {
                EventKind::Down => "1",
                EventKind::Up => "0",
            };
            let title = event_title(ev);
            let message = event_text(ev);
            let resp = self
                .client
                .post(&url)
                .form(&[
                    ("token", self.token.as_str()),
                    ("user", self.user.as_str()),
                    ("title", title.as_str()),
                    ("message", message.as_str()),
                    ("priority", priority),
                ])
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

/// Read a required non-empty string field from parsed channel config.
fn cfg_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Build a notifier for a channel from its `(kind, config_json)`. Returns
/// `None` (with a warning) when a required config field is missing or blank —
/// `deliver_event` skips such channels rather than failing the event.
pub fn notifier_for(channel: &Channel) -> Option<Box<dyn Notifier>> {
    let cfg: serde_json::Value = serde_json::from_str(&channel.config_json)
        .map_err(|e| {
            tracing::warn!(channel_id = channel.id, "invalid config_json: {e}");
        })
        .ok()?;
    let missing = |field: &str| {
        tracing::warn!(
            channel_id = channel.id,
            kind = channel.kind.as_str(),
            "channel missing required config field: {field}"
        );
        None::<Box<dyn Notifier>>
    };
    match channel.kind {
        ChannelKind::Webhook => match cfg_str(&cfg, "url") {
            Some(url) => Some(Box::new(WebhookNotifier::new(url))),
            None => missing("url"),
        },
        ChannelKind::Slack => match cfg_str(&cfg, "url") {
            Some(url) => Some(Box::new(SlackNotifier::new(url))),
            None => missing("url"),
        },
        ChannelKind::Telegram => match (cfg_str(&cfg, "token"), cfg_str(&cfg, "chat_id")) {
            (Some(token), Some(chat_id)) => Some(Box::new(TelegramNotifier::new(token, chat_id))),
            _ => missing("token/chat_id"),
        },
        ChannelKind::Ntfy => match cfg_str(&cfg, "topic") {
            Some(topic) => {
                let base_url =
                    cfg_str(&cfg, "base_url").unwrap_or_else(|| "https://ntfy.sh".to_string());
                let token = cfg_str(&cfg, "token");
                Some(Box::new(NtfyNotifier::new(base_url, topic, token)))
            }
            None => missing("topic"),
        },
        ChannelKind::Pushover => match (cfg_str(&cfg, "token"), cfg_str(&cfg, "user")) {
            (Some(token), Some(user)) => Some(Box::new(PushoverNotifier::new(token, user))),
            _ => missing("token/user"),
        },
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
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn telegram_posts_sendmessage_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendMessage"))
            .and(body_string_contains("\"chat_id\":\"999\""))
            .and(body_string_contains("DOWN"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"ok\":true}"))
            .expect(1)
            .mount(&server)
            .await;

        let n = TelegramNotifier::with_base_url("123:ABC".into(), "999".into(), server.uri());
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

    #[tokio::test]
    async fn telegram_returns_err_on_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("{\"ok\":false}"))
            .mount(&server)
            .await;
        let n = TelegramNotifier::with_base_url("bad".into(), "1".into(), server.uri());
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
        };
        assert!(n.send(&ev).await.is_err());
    }

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

    #[tokio::test]
    async fn slack_posts_text_json() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/services/hook"))
            .and(body_string_contains("\"text\""))
            .and(body_string_contains("UP"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .expect(1)
            .mount(&server)
            .await;

        let n = SlackNotifier::new(format!("{}/services/hook", server.uri()));
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Up,
            at: Utc::now(),
            project_id: 1,
        };
        n.send(&ev).await.unwrap();
    }

    #[tokio::test]
    async fn slack_returns_err_on_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let n = SlackNotifier::new(format!("{}/x", server.uri()));
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Up,
            at: Utc::now(),
            project_id: 1,
        };
        assert!(n.send(&ev).await.is_err());
    }

    #[tokio::test]
    async fn ntfy_posts_body_with_headers_and_token() {
        use wiremock::matchers::header;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/mytopic"))
            .and(header("authorization", "Bearer tok"))
            .and(header("priority", "high"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"id\":\"x\"}"))
            .expect(1)
            .mount(&server)
            .await;

        let n = NtfyNotifier::new(server.uri(), "mytopic".into(), Some("tok".into()));
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
        };
        n.send(&ev).await.unwrap();
    }

    #[tokio::test]
    async fn ntfy_works_without_token_and_errors_on_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let n = NtfyNotifier::new(server.uri(), "t".into(), None);
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Up,
            at: Utc::now(),
            project_id: 1,
        };
        assert!(n.send(&ev).await.is_err());
    }

    #[tokio::test]
    async fn pushover_posts_form_with_token_and_user() {
        use wiremock::matchers::body_string_contains;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/1/messages.json"))
            .and(body_string_contains("token=apptok"))
            .and(body_string_contains("user=userkey"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"status\":1}"))
            .expect(1)
            .mount(&server)
            .await;

        let n = PushoverNotifier::with_base_url("apptok".into(), "userkey".into(), server.uri());
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
        };
        n.send(&ev).await.unwrap();
    }

    #[tokio::test]
    async fn pushover_returns_err_on_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("{\"status\":0}"))
            .mount(&server)
            .await;
        let n = PushoverNotifier::with_base_url("bad".into(), "bad".into(), server.uri());
        let ev = NotificationEvent {
            check_id: 1,
            check_name: "backup".into(),
            event: EventKind::Up,
            at: Utc::now(),
            project_id: 1,
        };
        assert!(n.send(&ev).await.is_err());
    }

    fn channel_with(kind: ChannelKind, config_json: &str) -> Channel {
        Channel {
            id: 1,
            project_id: 1,
            kind,
            name: "c".into(),
            config_json: config_json.into(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn notifier_for_builds_each_kind_with_valid_config() {
        assert!(notifier_for(&channel_with(
            ChannelKind::Webhook,
            "{\"url\":\"http://x\"}"
        ))
        .is_some());
        assert!(
            notifier_for(&channel_with(ChannelKind::Slack, "{\"url\":\"http://x\"}")).is_some()
        );
        assert!(notifier_for(&channel_with(
            ChannelKind::Telegram,
            "{\"token\":\"t\",\"chat_id\":\"1\"}"
        ))
        .is_some());
        assert!(notifier_for(&channel_with(
            ChannelKind::Ntfy,
            "{\"base_url\":\"https://ntfy.sh\",\"topic\":\"t\"}"
        ))
        .is_some());
        assert!(notifier_for(&channel_with(
            ChannelKind::Pushover,
            "{\"token\":\"t\",\"user\":\"u\"}"
        ))
        .is_some());
    }

    #[test]
    fn notifier_for_returns_none_on_missing_config() {
        assert!(notifier_for(&channel_with(ChannelKind::Slack, "{}")).is_none());
        assert!(notifier_for(&channel_with(ChannelKind::Telegram, "{\"token\":\"t\"}")).is_none());
        assert!(notifier_for(&channel_with(ChannelKind::Ntfy, "{\"base_url\":\"x\"}")).is_none());
        assert!(notifier_for(&channel_with(ChannelKind::Pushover, "{\"token\":\"t\"}")).is_none());
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
