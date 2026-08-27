//! The browser session, and the emulations the suite depends on.
//!
//! `WebDriver::managed` downloads and supervises a matching chromedriver, but
//! not the browser: a local Chrome or Chromium is a prerequisite.
//! [`Browser::open`] says so explicitly, because the raw driver error does not.
//!
//! Every emulation goes through CDP rather than `BiDi`, which has no equivalent
//! of `Emulation.setEmulatedMedia` (`prefers-color-scheme`) or
//! `Emulation.setScriptExecutionDisabled` (`no_js.feature`).

use std::time::Duration;

use anyhow::{Context, Result};
use thirtyfour::prelude::*;

/// How long a query waits for a condition before giving up.
///
/// Only paid in full by a genuine failure, so it is sized for the slowest
/// machine: a two-core CI runner driving several browsers took over 10 s to
/// land a navigation.
pub const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

/// How often a query re-checks while waiting.
pub const WAIT_INTERVAL: Duration = Duration::from_millis(100);

/// A viewport, in CSS pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
}

impl Viewport {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

/// The default viewport.
pub const DESKTOP: Viewport = Viewport::new(1280, 720);

/// Whether the page's own scripts run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scripting {
    /// `assets/theme-init.js` and `assets/app.js` run.
    Enabled,
    /// The `@nojs` path: the page's own scripts never execute.
    Disabled,
}

/// A browser session, scoped to one scenario.
#[derive(Debug)]
pub struct Browser {
    driver: WebDriver,
    viewport: Viewport,
}

impl Browser {
    /// Starts a headless session with the page's scripts on or off.
    ///
    /// # Errors
    ///
    /// Fails when no local browser is installed, when the driver cannot be
    /// downloaded, or when the session cannot be created.
    pub async fn open(scripting: Scripting) -> Result<Self> {
        let mut caps = DesiredCapabilities::chrome();
        // `app.js`'s destructive forms go through `confirm()`, and the suite
        // answers some yes and one no. WebDriver's default of "dismiss and
        // notify" would silently cancel every delete; "ignore" leaves the
        // prompt standing for `Dom::accept_confirm` / `Dom::dismiss_confirm`.
        // Alert commands stay exempt, so anything else issued while a prompt is
        // open fails loudly — the right outcome for an unexpected dialog.
        caps.set("unhandledPromptBehavior", "ignore")?;
        caps.add_arg("--headless=new")?;
        caps.add_arg(&format!(
            "--window-size={},{}",
            DESKTOP.width, DESKTOP.height
        ))?;
        // Containers get a 64 MB /dev/shm by default, which Chrome outgrows.
        caps.add_arg("--disable-dev-shm-usage")?;
        // Without this, Linux's classic scrollbars take 15px out of the
        // viewport while macOS's overlay scrollbars take none, so a
        // mobile-layout width comparison passes locally and fails on CI.
        caps.add_arg("--hide-scrollbars")?;

        let driver = WebDriver::managed(caps).await.context(
            "could not start a browser session — a local Chrome or Chromium is required \
             (`brew install --cask ungoogled-chromium`, or `google-chrome` on CI); \
             unlike Playwright, the driver manager downloads only the driver",
        )?;

        let mut browser = Self {
            driver,
            viewport: DESKTOP,
        };
        // `--window-size` sizes the *window*; the stylesheet reads the
        // viewport, and the two differ by the platform's headless chrome.
        // `assets/app.css` branches at 720px, 640px and 560px, so a desktop
        // scenario laid out narrower than it asked for would silently be tested
        // against the phone stylesheet.
        browser.set_viewport(DESKTOP).await?;
        if scripting == Scripting::Disabled {
            browser.disable_scripting().await?;
        }
        Ok(browser)
    }

    /// Downloads and starts the driver once, before any scenario asks for it.
    ///
    /// `WebDriver::managed` builds a new manager per call, so on a cold cache
    /// (every CI run) parallel sessions all download the same driver and stall
    /// on its lock file. Opening and closing one session up front settles it.
    ///
    /// # Errors
    ///
    /// Fails for the same reasons [`Browser::open`] does.
    pub async fn prepare() -> Result<()> {
        Self::open(Scripting::Enabled).await?.quit().await
    }

    /// The underlying session, for the steps.
    pub fn driver(&self) -> &WebDriver {
        &self.driver
    }

    /// The viewport the session is currently emulating.
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Resizes the viewport.
    ///
    /// Uses `Emulation.setDeviceMetricsOverride` rather than the `WebDriver`
    /// window commands: a headless window's outer size includes chrome the
    /// layout does not see, and `mobile_layout.feature` asserts exact
    /// breakpoints.
    ///
    /// # Errors
    ///
    /// Fails when the CDP command is refused.
    pub async fn set_viewport(&mut self, viewport: Viewport) -> Result<()> {
        self.driver
            .cdp()
            .send_raw(
                "Emulation.setDeviceMetricsOverride",
                serde_json::json!({
                    "width": viewport.width,
                    "height": viewport.height,
                    "deviceScaleFactor": 1,
                    "mobile": false,
                }),
            )
            .await?;
        self.viewport = viewport;
        Ok(())
    }

    /// Emulates `prefers-color-scheme`.
    ///
    /// `theme.feature` and the scriptless half of `no_js.feature` turn on it:
    /// with no stored preference and no script, `app.css`'s
    /// `@media (prefers-color-scheme: light) { :root:not([data-theme]) }` is
    /// the only thing that answers.
    ///
    /// # Errors
    ///
    /// Fails when the CDP command is refused.
    pub async fn emulate_color_scheme(&self, scheme: &str) -> Result<()> {
        self.driver
            .cdp()
            .send_raw(
                "Emulation.setEmulatedMedia",
                serde_json::json!({
                    "media": "screen",
                    "features": [{ "name": "prefers-color-scheme", "value": scheme }],
                }),
            )
            .await?;
        Ok(())
    }

    /// Sends an extra header with every request.
    ///
    /// `account.feature` presents an `X-Forwarded-For` that `auth::client_ip`
    /// only honours alongside the `@trusted-proxy` tag.
    ///
    /// # Errors
    ///
    /// Fails when the CDP command is refused.
    pub async fn set_extra_headers(&self, headers: serde_json::Value) -> Result<()> {
        // `Network.setExtraHTTPHeaders` is only honoured once the Network
        // domain is enabled, and unlike the Emulation commands it does not
        // enable it implicitly.
        self.driver
            .cdp()
            .send_raw("Network.enable", serde_json::json!({}))
            .await?;
        self.driver
            .cdp()
            .send_raw(
                "Network.setExtraHTTPHeaders",
                serde_json::json!({ "headers": headers }),
            )
            .await?;
        Ok(())
    }

    /// Grants clipboard access; without it `navigator.clipboard.writeText`
    /// rejects headlessly and the copy button never reaches its copied state.
    ///
    /// # Errors
    ///
    /// Fails when the CDP command is refused.
    pub async fn grant_clipboard(&self) -> Result<()> {
        self.driver
            .cdp()
            .send_raw(
                "Browser.grantPermissions",
                serde_json::json!({
                    "permissions": ["clipboardReadWrite", "clipboardSanitizedWrite"],
                }),
            )
            .await?;
        Ok(())
    }

    /// Ends the session.
    ///
    /// # Errors
    ///
    /// Fails when the driver refuses to close.
    pub async fn quit(self) -> Result<()> {
        self.driver.quit().await?;
        Ok(())
    }

    /// Stops the page's own scripts from running.
    ///
    /// Takes effect on the *next* document, so it is issued before the first
    /// navigation — which is why sessions are per-scenario rather than shared.
    async fn disable_scripting(&self) -> Result<()> {
        self.driver
            .cdp()
            .send_raw(
                "Emulation.setScriptExecutionDisabled",
                serde_json::json!({ "value": true }),
            )
            .await?;
        Ok(())
    }
}
