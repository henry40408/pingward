//! States only the scan loop produces. `@fast-scan` sets
//! `PINGWARD_SCAN_INTERVAL=1`: the loop's first sleep is sized at boot, before
//! any check exists, so without it these wait out the 30s env default.

use anyhow::Result;
use cucumber::{then, when};
use pingward_e2e::dom::Dom;
use pingward_e2e::wait::eventually_within;
use pingward_e2e::world::PingwardWorld;
use std::time::Duration;

/// An outer bound; the transition normally lands in about two seconds.
const TRANSITION_TIMEOUT: Duration = Duration::from_secs(30);

#[when("I create a check that falls due almost immediately")]
async fn create_soon_due_check(world: &mut PingwardWorld) -> Result<()> {
    // Period 1s, grace 0 (the form pre-fills 300).
    let driver = world.driver()?;
    driver.submit("new-check-link").await?;
    driver.fill("check-name-input", "overdue-job").await?;
    driver.fill("check-period-input", "1").await?;
    driver.fill_css("#grace_secs", "0").await?;
    driver.submit("check-submit").await?;
    world.expect_path_matching(r"/checks/\d+$").await
}

#[when(expr = "I create a check with a {int} second max runtime")]
async fn create_max_runtime_check(world: &mut PingwardWorld, seconds: i64) -> Result<()> {
    // A long period rules out *overdue*, leaving only the overrun path.
    let driver = world.driver()?;
    driver.submit("new-check-link").await?;
    driver.fill("check-name-input", "overrun-job").await?;
    driver.fill("check-period-input", "3600").await?;
    driver.fill_css("#grace_secs", "0").await?;
    driver
        .fill_css("#max_runtime_secs", &seconds.to_string())
        .await?;
    driver.submit("check-submit").await?;
    world.expect_path_matching(r"/checks/\d+$").await
}

#[then("the check status eventually becomes down")]
async fn status_eventually_down(world: &mut PingwardWorld) -> Result<()> {
    // Server-side transition, so poll by reloading.
    let driver = world.driver()?;
    eventually_within(TRANSITION_TIMEOUT, "the check goes down", || async {
        driver.refresh().await?;
        Ok(driver.text_of_test_id("check-status").await? == Some("down".to_owned()))
    })
    .await
}
