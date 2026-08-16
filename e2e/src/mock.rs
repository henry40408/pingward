//! The endpoint the webhook channels in a scenario are pointed at.
//!
//! Replaces `support/mock-http.js`, a hand-rolled `node:http` recorder, with
//! `wiremock` — already a dev-dependency of the root crate, so it is not a new
//! name in the tree. It answers 200 to everything, which is what makes the
//! notifier record a successful send; the scenarios that want a *failed*
//! delivery point the channel somewhere nothing is listening instead.
//!
//! Delivery is fire-and-forget in pingward (`notify::deliver_event` is spawned
//! so a ping response is never blocked on notification I/O), so the POST lands
//! shortly *after* the response the step was waiting on — hence
//! [`MockWebhook::wait_for_payload`] polls rather than reads once.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use wiremock::matchers::any;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// How long to wait for a delivery that is already in flight.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(15);

/// How often to re-read the recorded requests while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// An HTTP endpoint that records every request it receives.
#[derive(Debug)]
pub struct MockWebhook {
    server: MockServer,
}

impl MockWebhook {
    /// Starts a receiver on an ephemeral port.
    ///
    /// # Errors
    ///
    /// Fails when the catch-all rule cannot be registered.
    pub async fn start() -> Result<Self> {
        let server = MockServer::start().await;
        Mock::given(any())
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&server)
            .await;
        Ok(Self { server })
    }

    /// The URL a channel should be configured with.
    pub fn url(&self) -> String {
        self.server.uri()
    }

    /// Every JSON body received so far, in arrival order.
    ///
    /// Bodies that are not JSON are skipped rather than failing the read: the
    /// recorder is deliberately indiscriminate about what it accepts.
    ///
    /// # Errors
    ///
    /// Fails when the recorder is not retaining requests.
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

    /// Waits for a received payload whose `event` field is `event`, and hands
    /// it back.
    ///
    /// # Errors
    ///
    /// Fails when no such payload arrives within [`DELIVERY_TIMEOUT`].
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

    /// Asserts that nothing has been delivered.
    ///
    /// # Errors
    ///
    /// Fails when anything was received.
    pub async fn expect_nothing(&self) -> Result<()> {
        let payloads = self.payloads().await?;
        if !payloads.is_empty() {
            bail!("expected no delivery, but {} arrived", payloads.len());
        }
        Ok(())
    }
}
