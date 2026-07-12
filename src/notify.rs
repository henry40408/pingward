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

#[derive(Debug, Clone)]
pub struct NotificationEvent {
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
            client: reqwest::Client::new(),
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
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
        };
        n.send(&ev).await.unwrap();
        // wiremock verifies expect(1) on drop
    }
}
