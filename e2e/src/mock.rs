//! Webhook receiver for notification scenarios. Answers 200 to everything
//! (failed-delivery scenarios point elsewhere). Delivery is fire-and-forget,
//! so [`MockWebhook::wait_for_payload`] polls.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(15);

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// An HTTP endpoint that records every request it receives.
#[derive(Debug)]
pub struct MockWebhook {
    server: MockServer,
}

impl MockWebhook {
    pub async fn start() -> Result<Self> {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        Ok(Self { server })
    }

    pub fn url(&self) -> String {
        self.server.uri()
    }

    /// Every JSON body received so far, in order; non-JSON bodies are skipped.
    pub async fn payloads(&self) -> Result<Vec<serde_json::Value>> {
        let requests = self
            .server
            .received_requests()
            .await
            .context("the mock receiver is not recording requests")?;
        Ok(requests
            .iter()
            .filter_map(|request| serde_json::from_slice(&request.body).ok())
            .collect())
    }

    /// Waits for a payload whose `event` field is `event`.
    pub async fn wait_for_payload(&self, event: &str) -> Result<serde_json::Value> {
        let deadline = Instant::now() + DELIVERY_TIMEOUT;
        loop {
            let payloads = self.payloads().await?;
            if let Some(found) = payloads
                .iter()
                .find(|payload| payload.get("event").and_then(|v| v.as_str()) == Some(event))
            {
                return Ok(found.clone());
            }
            if Instant::now() >= deadline {
                let seen: Vec<&str> = payloads
                    .iter()
                    .map(|payload| {
                        payload
                            .get("event")
                            .and_then(|v| v.as_str())
                            .unwrap_or("<no event>")
                    })
                    .collect();
                bail!(
                    "no `{event}` notification arrived within {DELIVERY_TIMEOUT:?}; \
                     received {seen:?}"
                );
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    pub async fn expect_nothing(&self) -> Result<()> {
        let payloads = self.payloads().await?;
        if !payloads.is_empty() {
            bail!("expected no delivery, but {} arrived", payloads.len());
        }
        Ok(())
    }
}
