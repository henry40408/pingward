//! The browser session, and the emulations the suite depends on.
//!
//! `WebDriver::managed` downloads and supervises a matching chromedriver
//! itself, so nothing has to be installed alongside the tests — but it does
//! *not* download the browser, unlike the Playwright setup this replaces. A
//! Chrome or Chromium in one of the well-known locations is a prerequisite now;
//! [`Browser::open`] says so in as many words when it is missing, because the
//! raw driver error does not.
//!
//! Every emulation goes through CDP rather than `BiDi`.
//! `Emulation.setEmulatedMedia` is the only way to reach `prefers-color-scheme`
//! at all — `BiDi` has no equivalent — and `Emulation.setScriptExecutionDisabled`
//! is what Playwright's `javaScriptEnabled: false` did underneath, so the
//! `no_js.feature` scenarios run against the same mechanism as before.

use std::time::Duration;

use anyhow::{Context, Result};
use thirtyfour::prelude::*;

/// How long a query waits for a condition before giving up.
///
/// Only ever paid in full by a genuine failure, so it is set for the slowest
/// machine that runs this rather than the fastest: locally every wait settles
/// in well under a second, while a two-core CI runner driving several browsers
/// took longer than 10 s to land a navigation.
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

/// The default viewport, matching the `Desktop Chrome` device both Playwright
/// projects used.
pub const DESKTOP: Viewport = Viewport::new(1280, 720);

/// Whether the page's own scripts run — the `chromium` / `no-js` project split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scripting {
    /// The scripted path: `assets/theme-init.js` and `assets/app.js` run.
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
        // `app.js`'s destructive forms go through `confirm()`, and this suite
        // needs to answer some of them yes and one of them no. WebDriver's
        // default is "dismiss and notify", which would silently cancel every
        // delete; Playwright's default was the same, which is why the old
        // steps registered a `page.on("dialog", …)` handler each time.
        //
        // "ignore" leaves the prompt standing so the step can answer it
        // explicitly (see `Dom::accept_confirm` / `Dom::dismiss_confirm`). The
        // Alert commands are exempt from the prompt-handling step, so they
        // still work; anything *else* issued while a prompt is open fails
        // loudly, which is the right outcome for a dialog nobody expected.
        caps.set("unhandledPromptBehavior", "ignore")?;
        caps.add_arg("--headless=new")?;
        caps.add_arg(&format!(
            "--window-size={},{}",
            DESKTOP.width, DESKTOP.height
        ))?;
        // Containers get a 64 MB /dev/shm by default, which Chrome outgrows.
        caps.add_arg("--disable-dev-shm-usage")?;
        // Playwright launched chromium with this, and the mobile-layout
        // assertions were written against it. Without it the classic
        // scrollbars on Linux take 15px out of the viewport, while macOS's
        // overlay scrollbars take none — which makes a width comparison pass
        // locally and fail only on CI.
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
        // `--window-size` above sizes the *window*; what the stylesheet reads
        // is the viewport, and the two differ by whatever chrome the platform's
        // headless build keeps. Pinning it here is what Playwright's `viewport`
        // option did, and `assets/app.css` branches at 720px, 640px and 560px —
        // a desktop scenario laid out a little narrower than it asked for would
        // silently be tested against the phone stylesheet.
        browser.set_viewport(DESKTOP).await?;
        if scripting == Scripting::Disabled {
            browser.disable_scripting().await?;
        }
        Ok(browser)
    }

    /// Downloads and starts the driver once, before any scenario asks for it.
    ///
    /// `WebDriver::managed` builds a *new* manager per call, so each session
    /// prepares the driver for itself. That is harmless when it is already
    /// cached and pathological when it is not: several sessions opening at once
    /// on a cold cache all try to download the same driver and contend on its
    /// lock file, which is a stall, not a slowdown. CI has a cold cache every
    /// run, which is exactly where the scenarios run in parallel.
    ///
    /// One session opened and closed up front settles it — the download happens
    /// once, and every later session finds the driver in place.
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

    /// Resizes the viewport, Playwright's `setViewportSize`.
    ///
    /// Goes through `Emulation.setDeviceMetricsOverride` rather than the
    /// `WebDriver` window commands: a headless window's outer size includes
    /// chrome the layout does not see, so setting 375×667 that way lands a
    /// viewport of some other width — and `mobile_layout.feature` asserts on
    /// exact breakpoints.
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

    /// Emulates `prefers-color-scheme`, Playwright's
    /// `emulateMedia({ colorScheme })`.
    ///
    /// `theme.feature` and the scriptless half of `no_js.feature` both turn on
    /// this: with no stored preference and no script, `app.css`'s
    /// `@media (prefers-color-scheme: light) { :root:not([data-theme]) }` is the
    /// only thing that answers.
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

    /// Sends an extra header with every request, Playwright's
    /// `setExtraHTTPHeaders`.
    ///
    /// `account.feature` needs it to present an `X-Forwarded-For` that
    /// `auth::client_ip` will honour — which only means anything alongside the
    /// `@trusted-proxy` tag, since an untrusted caller's header is ignored by
    /// design.
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

    /// Grants clipboard access, Playwright's
    /// `grantPermissions(["clipboard-read", "clipboard-write"])`.
    ///
    /// Without it `navigator.clipboard.writeText` rejects in a headless
    /// browser, and the ping URL's copy button never reaches its copied state.
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
