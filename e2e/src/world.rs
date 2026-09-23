//! The Cucumber world: one server, database and browser per scenario. They
//! are opened by the `before` hook, not `World::new`, because both depend on
//! the scenario's tags.

use anyhow::{Context, Result};
use cucumber::World;
use thirtyfour::prelude::*;

use crate::api::Api;
use crate::browser::{Browser, Scripting, Viewport};
use crate::mock::MockWebhook;
use crate::server::{Options, Server};

/// State shared by the steps of one scenario.
#[derive(Debug, World)]
#[world(init = Self::new)]
pub struct PingwardWorld {
    server: Option<Server>,
    browser: Option<Browser>,
    mock_webhook: Option<MockWebhook>,
    pub project_url: Option<String>,
    pub project_id: Option<i64>,
    pub check_id: Option<i64>,
    pub ping_url: Option<String>,
    pub webhook_url: Option<String>,
    /// Status of the last out-of-band HTTP request.
    pub status: Option<u16>,
    /// Status of the last ping, for steps about the response itself.
    pub ping_status: Option<u16>,
    /// Page text saved for a before/after comparison.
    pub remembered: Option<String>,
}

impl PingwardWorld {
    #[allow(clippy::unnecessary_wraps)]
    fn new() -> Result<Self> {
        Ok(Self {
            server: None,
            browser: None,
            mock_webhook: None,
            project_url: None,
            project_id: None,
            check_id: None,
            ping_url: None,
            webhook_url: None,
            status: None,
            ping_status: None,
            remembered: None,
        })
    }

    pub async fn open(&mut self, options: &Options, scripting: Scripting) -> Result<()> {
        self.server = Some(Server::start(options).await?);
        self.browser = Some(Browser::open(scripting).await?);
        Ok(())
    }

    pub async fn close(&mut self) -> Result<()> {
        if let Some(browser) = self.browser.take() {
            browser.quit().await?;
        }
        self.mock_webhook = None;
        self.server = None;
        Ok(())
    }

    pub fn browser(&self) -> Result<&Browser> {
        self.browser
            .as_ref()
            .context("no browser session: the `before` hook did not open one")
    }

    pub fn browser_mut(&mut self) -> Result<&mut Browser> {
        self.browser
            .as_mut()
            .context("no browser session: the `before` hook did not open one")
    }

    pub fn driver(&self) -> Result<&WebDriver> {
        Ok(self.browser()?.driver())
    }

    pub fn base_url(&self) -> Result<&str> {
        Ok(self
            .server
            .as_ref()
            .context("no server: the `before` hook did not start one")?
            .base_url())
    }

    pub fn api(&self) -> Result<Api> {
        Api::new(self.base_url()?)
    }

    /// The scenario's webhook receiver, started lazily since most scenarios
    /// never deliver anything.
    pub async fn mock_webhook(&mut self) -> Result<&MockWebhook> {
        if self.mock_webhook.is_none() {
            self.mock_webhook = Some(MockWebhook::start().await?);
        }
        Ok(self
            .mock_webhook
            .as_ref()
            .expect("just started the receiver above"))
    }

    pub async fn goto(&self, path: &str) -> Result<()> {
        let url = format!("{}{path}", self.base_url()?);
        self.driver()?.goto(&url).await?;
        Ok(())
    }

    /// The current URL's path and query.
    pub async fn path(&self) -> Result<String> {
        let url = self.driver()?.current_url().await?;
        Ok(match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        })
    }

    /// Waits for the path and query to equal `expected`.
    pub async fn expect_path(&self, expected: &str) -> Result<()> {
        crate::wait::eventually_eq(&format!("URL is {expected}"), expected.to_owned(), || {
            self.path()
        })
        .await
    }

    /// Waits for the path and query to match the regex `pattern`.
    pub async fn expect_path_matching(&self, pattern: &str) -> Result<()> {
        let regex = regex::Regex::new(pattern)?;
        crate::wait::eventually(&format!("URL matches {pattern}"), || async {
            Ok(regex.is_match(&self.path().await?))
        })
        .await
    }

    /// The status of a `fetch` issued by the page (`WebDriver` never reports
    /// one). It carries the session cookie but no CSRF token, so authz
    /// scenarios see the ownership 404 and CSRF scenarios the missing token.
    pub async fn fetch_status(&self, method: &str, path: &str) -> Result<u16> {
        let status = self
            .driver()?
            .execute_async(
                "const [path, method, done] = arguments;\
                 const init = { method, credentials: 'same-origin' };\
                 if (method !== 'GET') { init.body = new URLSearchParams(); }\
                 fetch(path, init).then((r) => done(r.status)).catch(() => done(0));",
                vec![
                    serde_json::json!(format!("{}{path}", self.base_url()?)),
                    serde_json::json!(method),
                ],
            )
            .await?;
        let status = status
            .json()
            .as_u64()
            .context("the fetch probe did not return a status")?;
        anyhow::ensure!(status != 0, "the request to {path} never completed");
        Ok(status as u16)
    }

    /// POSTs a form with the browser's cookies (caller supplies `_csrf`),
    /// following no redirects. Not via `fetch`: `redirect: 'manual'` reads a
    /// 303 as 0, and self-guard scenarios must tell it from a `csrf_guard` 403.
    pub async fn post_form_as_user(&self, path: &str, form: &[(&str, &str)]) -> Result<u16> {
        let jar = self
            .driver()?
            .get_all_cookies()
            .await?
            .iter()
            .map(|cookie| format!("{}={}", cookie.name, cookie.value))
            .collect::<Vec<_>>()
            .join("; ");
        let response = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?
            .post(format!("{}{path}", self.base_url()?))
            .header(reqwest::header::COOKIE, jar)
            .form(form)
            .send()
            .await
            .with_context(|| format!("posting {path} as the signed-in user"))?;
        Ok(response.status().as_u16())
    }

    pub async fn resize(&mut self, viewport: Viewport) -> Result<()> {
        self.browser_mut()?.set_viewport(viewport).await
    }

    pub fn project_url(&self) -> Result<String> {
        self.project_url
            .clone()
            .context("no project: no step created one for this scenario")
    }

    pub fn ping_url(&self) -> Result<String> {
        self.ping_url
            .clone()
            .context("no ping URL: no step read one from a check page")
    }

    pub fn webhook_url(&self) -> Result<String> {
        self.webhook_url
            .clone()
            .context("no webhook URL: no step started the mock receiver")
    }
}
