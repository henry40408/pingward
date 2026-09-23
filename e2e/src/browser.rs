//! The browser session and its CDP emulations.
//!
//! `WebDriver::managed` downloads the driver but not the browser; a local
//! Chrome or Chromium is required. Emulations use CDP because `BiDi` lacks
//! `Emulation.setEmulatedMedia` and `Emulation.setScriptExecutionDisabled`.

use std::time::Duration;

use anyhow::{Context, Result};
use thirtyfour::prelude::*;

/// How long a query waits. Sized for a loaded two-core CI runner, where a
/// navigation can take over 10 s; only a real failure pays it in full.
pub const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

pub const WAIT_INTERVAL: Duration = Duration::from_millis(100);

/// In CSS pixels.
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

pub const DESKTOP: Viewport = Viewport::new(1280, 720);

/// Whether the page's own scripts run (`Disabled` for `@nojs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scripting {
    Enabled,
    Disabled,
}

/// One scenario's browser session.
#[derive(Debug)]
pub struct Browser {
    driver: WebDriver,
    viewport: Viewport,
}

impl Browser {
    /// Starts a headless session with the page's scripts on or off.
    pub async fn open(scripting: Scripting) -> Result<Self> {
        let mut caps = DesiredCapabilities::chrome();
        // The default "dismiss and notify" would silently cancel every
        // `confirm()`; "ignore" leaves it for `Dom::accept_confirm` /
        // `Dom::dismiss_confirm`, and any other command fails loudly meanwhile.
        caps.set("unhandledPromptBehavior", "ignore")?;
        caps.add_arg("--headless=new")?;
        caps.add_arg(&format!(
            "--window-size={},{}",
            DESKTOP.width, DESKTOP.height
        ))?;
        // Containers get a 64 MB /dev/shm by default, which Chrome outgrows.
        caps.add_arg("--disable-dev-shm-usage")?;
        // Linux scrollbars take 15px of viewport, macOS overlay ones none, so
        // width assertions would pass locally and fail on CI.
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
        // `--window-size` sizes the window, not the viewport; without this a
        // desktop scenario could fall under an `app.css` breakpoint
        // (720/640/560px) and silently test the phone layout.
        browser.set_viewport(DESKTOP).await?;
        // Headless Chrome inherits the host's appearance, so on a dark-mode
        // desktop the default 'system' theme would resolve dark. Pin light;
        // a scenario that wants dark re-issues `emulate_color_scheme`.
        browser.emulate_color_scheme("light").await?;
        if scripting == Scripting::Disabled {
            browser.disable_scripting().await?;
        }
        Ok(browser)
    }

    /// Opens and closes one session up front so the driver is downloaded once:
    /// `WebDriver::managed` builds a manager per call, and parallel sessions on
    /// a cold cache stall on the same download's lock file.
    pub async fn prepare() -> Result<()> {
        Self::open(Scripting::Enabled).await?.quit().await
    }

    pub fn driver(&self) -> &WebDriver {
        &self.driver
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// Resizes the viewport via CDP rather than `WebDriver` window commands,
    /// whose outer size includes chrome; `mobile_layout.feature` needs exact
    /// breakpoints.
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

    /// Emulates `prefers-color-scheme`: `open` pins "light", and `theme.feature`
    /// and `no_js.feature` switch it per scenario.
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

    /// Sends extra headers with every request (e.g. `X-Forwarded-For`, which
    /// `auth::client_ip` honours only under `@trusted-proxy`).
    pub async fn set_extra_headers(&self, headers: serde_json::Value) -> Result<()> {
        // `Network.setExtraHTTPHeaders` is ignored until the domain is enabled.
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

    /// Without this, headless `navigator.clipboard.writeText` rejects and the
    /// copy button never reaches its copied state.
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

    pub async fn quit(self) -> Result<()> {
        self.driver.quit().await?;
        Ok(())
    }

    /// Takes effect from the next document, so it must precede the first
    /// navigation; hence per-scenario sessions.
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
