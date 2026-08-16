//! Re-runnable README screenshot pipeline — a port of
//! `screenshots/capture.mjs`.
//!
//! ```text
//!   wipe DB -> boot #1 (migrations) -> POST /setup -> stop
//!   -> seed backdated demo history -> boot #2 -> log in -> capture -> stop
//! ```
//!
//! Run from `e2e/`:  `cargo run --bin screenshots`
//!
//! Two things the JavaScript version needed that this one does not. It shelled
//! out to the `sqlite3` CLI, an undeclared dependency; seeding now goes through
//! `sqlx`. And it stopped the server with SIGTERM so the WAL was checkpointed
//! before that CLI read the file — with a real `SQLite` client doing the
//! seeding, WAL recovery on open covers it, so the plain kill this can issue
//! without reaching for `libc` is enough.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use pingward_e2e::Api;
use pingward_e2e::browser::{Browser, Scripting, Viewport};
use pingward_e2e::dom::{Dom, TextContent, click_when_ready};
use pingward_e2e::seed::{ADMIN_PASSWORD, ADMIN_USERNAME, seed_sql};
use pingward_e2e::server::{free_port, repo_root};
use thirtyfour::WebElement;
use thirtyfour::prelude::*;
use tokio::process::{Child, Command};

/// The bind port is throwaway, but the *rendered* ping URLs come from
/// `PINGWARD_BASE_URL` — so this points at a plausible public hostname instead
/// of baking a random loopback port into the check-page screenshot.
const PUBLIC_BASE_URL: &str = "https://pingward.example.com";

/// Padding below the element a shot is cut at, so it ends on a card boundary
/// instead of slicing through a table.
const PAD: f64 = 16.0;

const DESKTOP: Device = Device {
    viewport: Viewport::new(1280, 900),
    scale: 2,
    mobile: false,
};

const MOBILE: Device = Device {
    viewport: Viewport::new(390, 844),
    scale: 3,
    mobile: true,
};

/// Freezes anything that would make two runs of the same page differ.
const FREEZE_CSS: &str =
    "*{animation:none !important;transition:none !important;caret-color:transparent !important}";

#[derive(Clone, Copy)]
struct Device {
    viewport: Viewport,
    scale: u32,
    mobile: bool,
}

/// A page region, named so the shot list reads as intent rather than as
/// selectors.
#[derive(Clone, Copy)]
enum Region {
    /// The card wrapping a selector — `.card:has(…)`.
    CardWith(&'static str),
    /// The card containing this text.
    CardSaying(&'static str),
    /// A plain CSS selector.
    Css(&'static str),
    /// The last element matching a selector.
    Last(&'static str),
    /// The nth element carrying a `data-testid`.
    NthTestId(&'static str, usize),
}

/// How much of the page a shot keeps.
#[derive(Clone, Copy)]
enum Frame {
    /// The whole page. Only right for the pages that end on their own — the
    /// check and admin pages are long enough that a whole-page capture reads
    /// as a strip in a README.
    Full,
    /// From the top down to just past a region. `pad` is 0 when the cut lands
    /// on a list divider, where any padding would leak a sliver of the next
    /// row.
    DownTo(Region, f64),
    /// The band spanned by two regions, for a shot of the middle of a long
    /// page.
    Band(Region, Region),
}

/// What to do after loading, to leave the page in the state worth
/// photographing.
#[derive(Clone, Copy)]
enum Settle {
    /// Wait for the nth dashboard row.
    DashboardRows(usize),
    /// Open the down check from the dashboard.
    DownCheck,
    /// Open it and expand the failed run, so its captured output is in frame —
    /// that body is what a `curl --data-binary @- …/fail` sends.
    DownCheckExpanded,
    /// Follow the first "Manage →" link.
    ManageProject,
    /// Wait for the admin scale tiles.
    AdminScale,
    /// Expand the newest audit entry, so the request and detail behind it —
    /// the half of `audit_log` that lives in the collapsed row — are in shot.
    AuditExpanded,
}

struct Shot {
    file: &'static str,
    scheme: &'static str,
    device: Device,
    goto: &'static str,
    settle: Settle,
    frame: Frame,
}

const SHOTS: [Shot; 9] = [
    Shot {
        file: "dashboard-dark.png",
        scheme: "dark",
        device: DESKTOP,
        goto: "/",
        settle: Settle::DashboardRows(9),
        frame: Frame::Full,
    },
    Shot {
        file: "check-dark.png",
        scheme: "dark",
        device: DESKTOP,
        goto: "/",
        settle: Settle::DownCheck,
        frame: Frame::DownTo(Region::CardSaying("Notify channels"), PAD),
    },
    Shot {
        file: "check-history-dark.png",
        scheme: "dark",
        device: DESKTOP,
        goto: "/",
        settle: Settle::DownCheckExpanded,
        frame: Frame::Band(Region::Css("#pings-card"), Region::Last(".card")),
    },
    Shot {
        file: "project-dark.png",
        scheme: "dark",
        device: DESKTOP,
        goto: "/",
        settle: Settle::ManageProject,
        frame: Frame::Full,
    },
    Shot {
        file: "admin-dark.png",
        scheme: "dark",
        device: DESKTOP,
        goto: "/admin",
        settle: Settle::AdminScale,
        frame: Frame::DownTo(Region::CardWith("[data-testid=\"sched-scan\"]"), PAD),
    },
    Shot {
        file: "admin-audit-dark.png",
        scheme: "dark",
        device: DESKTOP,
        goto: "/admin",
        settle: Settle::AuditExpanded,
        frame: Frame::Band(
            Region::CardWith("#audit-section"),
            Region::CardWith("#audit-section"),
        ),
    },
    Shot {
        file: "dashboard-light.png",
        scheme: "light",
        device: DESKTOP,
        goto: "/",
        settle: Settle::DashboardRows(9),
        frame: Frame::Full,
    },
    Shot {
        file: "dashboard-mobile.png",
        scheme: "dark",
        device: MOBILE,
        goto: "/",
        settle: Settle::DashboardRows(1),
        frame: Frame::DownTo(Region::NthTestId("dashboard-check-row", 1), 0.0),
    },
    Shot {
        file: "check-mobile.png",
        scheme: "dark",
        device: MOBILE,
        goto: "/",
        settle: Settle::DownCheck,
        frame: Frame::DownTo(Region::CardSaying("Notify channels"), PAD),
    },
];

#[tokio::main]
async fn main() -> Result<()> {
    let db = repo_root().join("e2e/.tmp/screenshots.sqlite3");
    let out = repo_root().join("docs/screenshots");
    std::fs::create_dir_all(db.parent().context("the database has a parent")?)?;
    std::fs::create_dir_all(&out)?;
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", db.display()));
    }

    let port = free_port()?;
    let base = format!("http://127.0.0.1:{port}");

    // Phase 1 — migrate, then create the first admin through the product's own
    // one-time setup form so the password hash is a real argon2 one. `/setup`
    // is CSRF-protected like every other POST (`csrf_guard` has no path
    // exemptions), so this goes through the same GET-then-submit helper the
    // E2E suite uses rather than posting the fields bare.
    let mut server = start_pingward(&db, port, &base).await?;
    let bootstrap = Api::new(&base)?
        .bootstrap_admin(ADMIN_USERNAME, ADMIN_PASSWORD)
        .await;
    stop_pingward(&mut server).await;
    bootstrap?;

    // Phase 2 — backdated demo data, written against the stopped database.
    apply_seed(&db).await?;

    // Phase 3 — boot on the seeded database and photograph it.
    let mut server = start_pingward(&db, port, &base).await?;
    let result = capture_all(&base, &out).await;
    stop_pingward(&mut server).await;
    result?;

    println!("done — PNGs in docs/screenshots");
    Ok(())
}

async fn capture_all(base: &str, out: &Path) -> Result<()> {
    let mut browser = Browser::open(Scripting::Enabled).await?;
    let result = capture_with(&mut browser, base, out).await;
    browser.quit().await?;
    result
}

async fn capture_with(browser: &mut Browser, base: &str, out: &Path) -> Result<()> {
    let driver = browser.driver().clone();
    // One session for the whole run, unlike the JavaScript version's context
    // per shot: the emulations that differed between contexts — viewport,
    // scale factor, colour scheme — are all CDP overrides that can simply be
    // re-issued, and sharing the session means signing in once instead of
    // replaying a stored cookie jar.
    driver
        .cdp()
        .send_raw(
            "Emulation.setTimezoneOverride",
            serde_json::json!({ "timezoneId": "UTC" }),
        )
        .await?;
    driver
        .cdp()
        .send_raw(
            "Emulation.setLocaleOverride",
            serde_json::json!({ "locale": "en-US" }),
        )
        .await?;

    driver.goto(format!("{base}/login")).await?;
    driver.fill("username-input", ADMIN_USERNAME).await?;
    driver.fill("password-input", ADMIN_PASSWORD).await?;
    driver.submit("login-submit").await?;
    driver.expect_visible("nav-admin").await?;

    for shot in &SHOTS {
        browser.set_viewport(shot.device.viewport).await?;
        set_device(&driver, shot.device).await?;
        emulate_media(&driver, shot.scheme).await?;

        driver.goto(format!("{base}{}", shot.goto)).await?;
        settle(&driver, shot.settle).await?;

        driver
            .execute(
                "const style = document.createElement('style');\
                 style.textContent = arguments[0];\
                 document.head.appendChild(style);",
                vec![serde_json::json!(FREEZE_CSS)],
            )
            .await?;
        driver
            .execute_async(
                "const done = arguments[0]; document.fonts.ready.then(() => done(true));",
                vec![],
            )
            .await?;

        let clip = frame_clip(&driver, shot).await?;
        let png = capture(&driver, &clip).await?;
        let path = out.join(shot.file);
        std::fs::write(&path, png).with_context(|| format!("writing {}", path.display()))?;
        println!("captured {}", shot.file);
    }
    Ok(())
}

/// Applies a device's metrics, Playwright's `deviceScaleFactor` / `isMobile` /
/// `hasTouch`.
async fn set_device(driver: &WebDriver, device: Device) -> Result<()> {
    driver
        .cdp()
        .send_raw(
            "Emulation.setDeviceMetricsOverride",
            serde_json::json!({
                "width": device.viewport.width,
                "height": device.viewport.height,
                "deviceScaleFactor": device.scale,
                "mobile": device.mobile,
            }),
        )
        .await?;
    driver
        .cdp()
        .send_raw(
            "Emulation.setTouchEmulationEnabled",
            serde_json::json!({ "enabled": device.mobile, "maxTouchPoints": 5 }),
        )
        .await?;
    Ok(())
}

/// Emulates the colour scheme *and* reduced motion in one command — the second
/// call would otherwise replace the first's feature list.
async fn emulate_media(driver: &WebDriver, scheme: &str) -> Result<()> {
    driver
        .cdp()
        .send_raw(
            "Emulation.setEmulatedMedia",
            serde_json::json!({
                "media": "screen",
                "features": [
                    { "name": "prefers-color-scheme", "value": scheme },
                    { "name": "prefers-reduced-motion", "value": "reduce" },
                ],
            }),
        )
        .await?;
    Ok(())
}

async fn settle(driver: &WebDriver, settle: Settle) -> Result<()> {
    match settle {
        Settle::DashboardRows(index) => {
            pingward_e2e::wait::eventually("the dashboard rows", || async {
                Ok(driver.test_ids("dashboard-check-row").await?.len() > index)
            })
            .await
        }
        Settle::DownCheck => open_down_check(driver).await,
        Settle::DownCheckExpanded => {
            open_down_check(driver).await?;
            driver.click_css("tr.toggle").await?;
            driver.expect_visible_css("tr.exp .out").await
        }
        Settle::ManageProject => {
            let link = driver
                .link_named("Manage →")
                .await?
                .context("the dashboard has no `Manage →` link")?;
            click_when_ready(&link).await?;
            driver.expect_visible("new-check-link").await
        }
        Settle::AdminScale => driver.expect_visible("admin-scale").await,
        Settle::AuditExpanded => {
            driver.expect_visible("audit-row").await?;
            driver.click_css("#audit-section tr.toggle").await?;
            driver
                .expect_visible_css("#audit-section tr.exp .out")
                .await
        }
    }
}

async fn open_down_check(driver: &WebDriver) -> Result<()> {
    driver
        .expect_exact_text_somewhere("home-nas-snapshot")
        .await?;
    let link = driver
        .link_named("home-nas-snapshot")
        .await?
        .context("no link to the down check")?;
    click_when_ready(&link).await?;
    driver.expect_visible("check-status").await?;
    driver.expect_visible("ping-row").await
}

/// A clip rectangle, in document coordinates.
struct Clip {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

async fn frame_clip(driver: &WebDriver, shot: &Shot) -> Result<Clip> {
    let width = f64::from(shot.device.viewport.width);
    Ok(match shot.frame {
        Frame::Full => Clip {
            x: 0.0,
            y: 0.0,
            width,
            height: document_height(driver).await?,
        },
        Frame::DownTo(region, pad) => {
            let box_ = region_box(driver, region).await?;
            Clip {
                x: 0.0,
                y: 0.0,
                width,
                height: (box_.1 + box_.3 + pad).ceil(),
            }
        }
        Frame::Band(from, to) => {
            let head = region_box(driver, from).await?;
            let tail = region_box(driver, to).await?;
            let y = (head.1 - PAD).max(0.0).floor();
            Clip {
                x: 0.0,
                y,
                width,
                height: (tail.1 + tail.3 + PAD - y).ceil(),
            }
        }
    })
}

async fn document_height(driver: &WebDriver) -> Result<f64> {
    driver
        .eval("return document.documentElement.scrollHeight;")
        .await?
        .as_f64()
        .context("the document height probe did not return a number")
}

/// A region's document-relative box, as `(x, y, width, height)`.
///
/// The page is scrolled back to the top first, so the viewport-relative box
/// the browser reports *is* the document box — which is the coordinate space a
/// beyond-the-viewport capture expects.
async fn region_box(driver: &WebDriver, region: Region) -> Result<(f64, f64, f64, f64)> {
    driver.execute("window.scrollTo(0, 0);", vec![]).await?;
    let element = resolve(driver, region).await?;
    let rect = driver
        .execute(
            "const r = arguments[0].getBoundingClientRect();\
             return [r.x, r.y, r.width, r.height];",
            vec![element.to_json()?],
        )
        .await?;
    let values: Vec<f64> = rect
        .json()
        .as_array()
        .context("the rect probe did not return an array")?
        .iter()
        .map(|value| value.as_f64().unwrap_or_default())
        .collect();
    let [x, y, width, height] = values[..] else {
        bail!(
            "the rect probe returned {} values, expected 4",
            values.len()
        );
    };
    Ok((x, y, width, height))
}

async fn resolve(driver: &WebDriver, region: Region) -> Result<WebElement> {
    match region {
        Region::CardWith(selector) => driver.css(&format!(".card:has({selector})")).await,
        Region::CardSaying(text) => {
            for card in driver.css_all(".card").await? {
                if card.content_text().await?.contains(text) {
                    return Ok(card);
                }
            }
            bail!("no card says {text:?}")
        }
        Region::Css(selector) => driver.css(selector).await,
        Region::Last(selector) => driver
            .css_all(selector)
            .await?
            .pop()
            .with_context(|| format!("nothing matches `{selector}`")),
        Region::NthTestId(id, index) => driver
            .test_ids(id)
            .await?
            .into_iter()
            .nth(index)
            .with_context(|| format!("fewer than {} elements carry `{id}`", index + 1)),
    }
}

/// Captures a clipped PNG.
///
/// `captureBeyondViewport` is what makes a clip taller than the window work —
/// Playwright's `fullPage` under a different name. The clip's own `scale`
/// stays 1: the device scale factor from the metrics override is already
/// applied, and multiplying the two would render the mobile shots at 9x.
async fn capture(driver: &WebDriver, clip: &Clip) -> Result<Vec<u8>> {
    let response = driver
        .cdp()
        .send_raw(
            "Page.captureScreenshot",
            serde_json::json!({
                "format": "png",
                "captureBeyondViewport": true,
                "clip": {
                    "x": clip.x,
                    "y": clip.y,
                    "width": clip.width,
                    "height": clip.height,
                    "scale": 1,
                },
            }),
        )
        .await?;
    let data = response
        .get("data")
        .and_then(serde_json::Value::as_str)
        .context("the capture returned no data")?;
    Ok(base64::engine::general_purpose::STANDARD.decode(data)?)
}

/// Runs the seed script against the stopped database.
async fn apply_seed(db: &Path) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    let sql = seed_sql(now)?;
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", db.display()))
        .await
        .with_context(|| format!("opening {}", db.display()))?;
    // Audited: every value in the script goes through `seed`'s own quoting, and
    // the whole thing is built from constants in this repository rather than
    // from anything a user supplies. That is what `AssertSqlSafe` is asking.
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(&pool)
        .await
        .context("running the seed script")?;
    pool.close().await;
    Ok(())
}

async fn start_pingward(db: &Path, port: u16, base: &str) -> Result<Child> {
    let binary = repo_root().join("target/debug/pingward");
    if !binary.is_file() {
        bail!(
            "{} is missing — run `cargo build` at the repository root first",
            binary.display()
        );
    }
    let child = Command::new(&binary)
        .current_dir(repo_root())
        .env(
            "DATABASE_URL",
            format!("sqlite://{}?mode=rwc", db.display()),
        )
        .env("PINGWARD_BIND", format!("127.0.0.1:{port}"))
        // The rendered ping URLs come from this, not from the bind address.
        .env("PINGWARD_BASE_URL", PUBLIC_BASE_URL)
        .env("RUST_LOG", "warn")
        .env("PINGWARD_SECRET", "pingward-screenshots-0123456789abcdef")
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(false)
        .spawn()
        .with_context(|| format!("spawning {}", binary.display()))?;

    let client = reqwest::Client::new();
    let healthz = format!("{base}/healthz");
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if let Ok(response) = client.get(&healthz).send().await
            && response.status().is_success()
        {
            return Ok(child);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("pingward did not answer {healthz} in time")
}

async fn stop_pingward(child: &mut Child) {
    let _ = child.kill().await;
}
