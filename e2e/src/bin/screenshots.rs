//! Re-runnable README screenshot pipeline.
//!
//! ```text
//!   wipe DB -> boot #1 (migrations) -> POST /setup -> stop
//!   -> seed backdated demo history -> boot #2 -> log in -> capture -> stop
//! ```
//!
//! Run from `e2e/`: `cargo run --bin screenshots`. The server is stopped with
//! a plain kill; `sqlx` recovers the WAL when seeding opens the database.

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

/// Rendered in ping URLs, keeping the random port out of the screenshots.
const PUBLIC_BASE_URL: &str = "https://pingward.example.com";

/// Padding past a shot's cut, so it ends on a card boundary.
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

/// Removes animation and caret nondeterminism.
const FREEZE_CSS: &str =
    "*{animation:none !important;transition:none !important;caret-color:transparent !important}";

#[derive(Clone, Copy)]
struct Device {
    viewport: Viewport,
    scale: u32,
    mobile: bool,
}

#[derive(Clone, Copy)]
enum Region {
    /// `.card:has(selector)`.
    CardWith(&'static str),
    /// The first card containing this text.
    CardSaying(&'static str),
    Css(&'static str),
    /// The last match.
    Last(&'static str),
    /// The nth (0-based) element with this `data-testid`.
    NthTestId(&'static str, usize),
}

/// How much of the page a shot keeps.
#[derive(Clone, Copy)]
enum Frame {
    /// Whole page; too long for the check and admin pages.
    Full,
    /// Top down to a region plus padding (0 on a list divider, or the next
    /// row leaks in).
    DownTo(Region, f64),
    /// From one region to another, for the middle of a long page.
    Band(Region, Region),
}

/// Post-load steps that put the page in the state to photograph.
#[derive(Clone, Copy)]
enum Settle {
    /// Wait for the dashboard row at this 0-based index.
    DashboardRows(usize),
    /// Open the down check from the dashboard.
    DownCheck,
    /// Also expand the newest failed run's captured output.
    DownCheckExpanded,
    /// Follow the first "Manage →" link.
    ManageProject,
    AdminScale,
    /// Expand the newest audit entry.
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

    // Phase 1: migrate, and create the admin via `/setup` for a real argon2 hash.
    let mut server = start_pingward(&db, port, &base).await?;
    let bootstrap = Api::new(&base)?
        .bootstrap_admin(ADMIN_USERNAME, ADMIN_PASSWORD)
        .await;
    stop_pingward(&mut server).await;
    bootstrap?;

    // Phase 2: seed the stopped database.
    apply_seed(&db).await?;

    // Phase 3: boot on it and capture.
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
    // One session for all shots (sign in once); device and scheme overrides
    // are re-issued per shot.
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

/// Both features in one call: a second call would replace the first's list.
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

/// In document coordinates.
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

/// A region's document box `(x, y, width, height)`: scrolling to the top
/// first makes the viewport-relative rect equal it.
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

/// Captures a clipped PNG. `captureBeyondViewport` allows clips taller than
/// the window; clip `scale` stays 1 since the device scale factor already
/// applies (3 x 3 would render mobile at 9x).
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

async fn apply_seed(db: &Path) -> Result<()> {
    let now = chrono::Utc::now().timestamp_millis();
    let sql = seed_sql(now)?;
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", db.display()))
        .await
        .with_context(|| format!("opening {}", db.display()))?;
    // Safe: built from in-repo constants, every value quoted by `seed`.
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
