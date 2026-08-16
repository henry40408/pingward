//! The Cucumber world: one server, one database and one browser session per
//! scenario.
//!
//! Neither the server nor the session can be opened in `World::new`, which
//! never sees the scenario: the server's environment comes from the tags
//! (`@fast-scan`, `@smtp-env`, `@trusted-proxy`) and whether the page's scripts
//! run comes from `@nojs`. A `before` hook opens both instead, which is also
//! the only order that works for scripting —
//! `Emulation.setScriptExecutionDisabled` applies to the next document, so it
//! has to be issued before the first navigation.
//!
//! `support/fixtures.js` built the same state out of test-scoped Playwright
//! fixtures, down to the scratch `world` object the steps hung remembered ids
//! on; those are the public fields here.

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
    /// The project page's path, remembered by the step that created it.
    pub project_url: Option<String>,
    /// The project's row id, parsed out of that path.
    pub project_id: Option<i64>,
    /// The check's row id, likewise.
    pub check_id: Option<i64>,
    /// The check's ping URL, once a step has read (or revealed) it.
    pub ping_url: Option<String>,
    /// The URL a scenario pointed a webhook channel at.
    pub webhook_url: Option<String>,
    /// The status code of the last out-of-band HTTP request a step made.
    pub status: Option<u16>,
    /// The status code of the last ping a step sent, when the step is about
    /// the ping's *response* rather than its effect.
    pub ping_status: Option<u16>,
    /// A remembered piece of page text, for the assertions that compare a
    /// value before and after an action.
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

    /// Starts this scenario's server and browser.
    ///
    /// # Errors
    ///
    /// Fails when the server does not come up, or when no browser session can
    /// be started.
    pub async fn open(&mut self, options: &Options, scripting: Scripting) -> Result<()> {
        self.server = Some(Server::start(options).await?);
        self.browser = Some(Browser::open(scripting).await?);
        Ok(())
    }

    /// Ends the session and stops the server.
    ///
    /// # Errors
    ///
    /// Fails when the driver refuses to close. The server and the mock
    /// receiver are torn down by their own `Drop`.
    pub async fn close(&mut self) -> Result<()> {
        if let Some(browser) = self.browser.take() {
            browser.quit().await?;
        }
        self.mock_webhook = None;
        self.server = None;
        Ok(())
    }

    /// The scenario's browser.
    ///
    /// # Errors
    ///
    /// Fails when no session was opened — a `before` hook that did not run.
    pub fn browser(&self) -> Result<&Browser> {
        self.browser
            .as_ref()
            .context("no browser session: the `before` hook did not open one")
    }

    /// The scenario's browser, mutably, for the emulations that latch.
    ///
    /// # Errors
    ///
    /// Fails when no session was opened.
    pub fn browser_mut(&mut self) -> Result<&mut Browser> {
        self.browser
            .as_mut()
            .context("no browser session: the `before` hook did not open one")
    }

    /// The scenario's driver.
    ///
    /// # Errors
    ///
    /// Fails when no session was opened.
    pub fn driver(&self) -> Result<&WebDriver> {
        Ok(self.browser()?.driver())
    }

    /// The base URL of the server under test.
    ///
    /// # Errors
    ///
    /// Fails when no server was started.
    pub fn base_url(&self) -> Result<&str> {
        Ok(self
            .server
            .as_ref()
            .context("no server: the `before` hook did not start one")?
            .base_url())
    }

    /// The HTTP helper for this scenario's server.
    ///
    /// # Errors
    ///
    /// Fails when no server was started.
    pub fn api(&self) -> Result<Api> {
        Api::new(self.base_url()?)
    }

    /// The mock receiver every webhook channel in this scenario points at,
    /// started on first use.
    ///
    /// Lazily, as the `mockWebhook` fixture was: most scenarios never deliver
    /// anything, and a receiver costs a listening socket.
    ///
    /// # Errors
    ///
    /// Fails when the receiver cannot bind.
    pub async fn mock_webhook(&mut self) -> Result<&MockWebhook> {
        if self.mock_webhook.is_none() {
            self.mock_webhook = Some(MockWebhook::start().await?);
        }
        Ok(self
            .mock_webhook
            .as_ref()
            .expect("just started the receiver above"))
    }

    /// Navigates to a path on the server under test.
    ///
    /// # Errors
    ///
    /// Fails when the navigation is refused.
    pub async fn goto(&self, path: &str) -> Result<()> {
        let url = format!("{}{path}", self.base_url()?);
        self.driver()?.goto(&url).await?;
        Ok(())
    }

    /// The current URL's path and query, the shape the steps assert against.
    ///
    /// # Errors
    ///
    /// Fails when the driver cannot report a URL.
    pub async fn path(&self) -> Result<String> {
        let url = self.driver()?.current_url().await?;
        Ok(match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        })
    }

    /// Waits for the browser to land on `expected`, Playwright's `toHaveURL`.
    ///
    /// Compares path and query rather than the whole URL: the server's port is
    /// ephemeral, so the old assertions' `${serverUrl}/…` has no stable form
    /// here.
    ///
    /// # Errors
    ///
    /// Fails when the URL has not settled on `expected` in time.
    pub async fn expect_path(&self, expected: &str) -> Result<()> {
        crate::wait::eventually_eq(&format!("URL is {expected}"), expected.to_owned(), || {
            self.path()
        })
        .await
    }

    /// Waits for the browser to land on a path matching `pattern`.
    ///
    /// The many `toHaveURL(/\/checks\/\d+$/)` assertions, which cannot name the
    /// id they are waiting for.
    ///
    /// # Errors
    ///
    /// Fails when the pattern does not compile, or the URL never matches.
    pub async fn expect_path_matching(&self, pattern: &str) -> Result<()> {
        let regex = regex::Regex::new(pattern)?;
        crate::wait::eventually(&format!("URL matches {pattern}"), || async {
            Ok(regex.is_match(&self.path().await?))
        })
        .await
    }

    /// Resizes the viewport.
    ///
    /// # Errors
    ///
    /// Fails when the CDP command is refused.
    pub async fn resize(&mut self, viewport: Viewport) -> Result<()> {
        self.browser_mut()?.set_viewport(viewport).await
    }

    /// The project page's path, as the creating step remembered it.
    ///
    /// # Errors
    ///
    /// Fails when no step created a project.
    pub fn project_url(&self) -> Result<String> {
        self.project_url
            .clone()
            .context("no project: no step created one for this scenario")
    }

    /// The check's ping URL.
    ///
    /// # Errors
    ///
    /// Fails when no step read one.
    pub fn ping_url(&self) -> Result<String> {
        self.ping_url
            .clone()
            .context("no ping URL: no step read one from a check page")
    }

    /// The URL a webhook channel was pointed at.
    ///
    /// # Errors
    ///
    /// Fails when no step configured one.
    pub fn webhook_url(&self) -> Result<String> {
        self.webhook_url
            .clone()
            .context("no webhook URL: no step started the mock receiver")
    }
}
