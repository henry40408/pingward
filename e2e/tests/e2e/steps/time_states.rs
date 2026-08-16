//! The states only the scan loop can produce — a port of
//! `time_states.steps.js`.
//!
//! Both scenarios run under `@fast-scan`, which starts the server with
//! `PINGWARD_SCAN_INTERVAL=1`. That is not a convenience: the loop's *first*
//! post-startup sleep is the env default whatever any per-check override says,
//! so without the tag these would wait out the full 30 s.

use anyhow::Result;
use cucumber::{then, when};
use pingward_e2e::dom::Dom;
use pingward_e2e::wait::eventually_within;
use pingward_e2e::world::PingwardWorld;
use std::time::Duration;

/// How long to keep reloading before giving up on the transition.
///
/// The scan loop lands it in about two seconds; this is the outer bound, not
/// the expectation.
const TRANSITION_TIMEOUT: Duration = Duration::from_secs(30);

#[when("I create a check that falls due almost immediately")]
async fn create_soon_due_check(world: &mut PingwardWorld) -> Result<()> {
    // Period 1s, grace 0 — so its due time is about a second after creation
    // and the scan loop downs it with no ping at all. `#grace_secs` has an id
    // but no `data-testid`, and the form pre-fills it to 300, so it has to be
    // overwritten.
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
    // A long period, so the check is never *overdue* during the scenario, plus
    // a short max runtime. A later `start` ping opens an in-flight run whose
    // deadline is start + max runtime, and the scan loop then downs it through
    // the overrun path only.
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
    // The scan loop runs in the pingward process, not in response to anything
    // the browser did, so this polls by reloading rather than waiting on the
    // open document.
    let driver = world.driver()?;
    eventually_within(TRANSITION_TIMEOUT, "the check goes down", || async {
        driver.refresh().await?;
        Ok(driver.text_of_test_id("check-status").await? == Some("down".to_owned()))
    })
    .await
}
