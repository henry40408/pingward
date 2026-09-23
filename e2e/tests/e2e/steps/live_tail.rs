//! The check page's opt-in SSE live tail. The feature's "without the live
//! tail" scenario is the control: its final reload proves the ping registered.

use std::time::Duration;

use anyhow::Result;
use cucumber::{then, when};
use pingward_e2e::dom::Dom;
use pingward_e2e::world::PingwardWorld;

#[when("I turn on the live tail")]
async fn turn_on_live_tail(world: &mut PingwardWorld) -> Result<()> {
    // Events publish only while `receiver_count() > 0`, with no catch-up, so a
    // ping before the `EventSource` opens is lost: wait for `data-live="open"`.
    let driver = world.driver()?;
    driver.click("pings-live").await?;
    driver
        .expect_attr("[data-testid=\"pings-live\"]", "data-live", Some("open"))
        .await
}

#[then("the recent pings table still shows no pings")]
async fn still_no_pings(world: &mut PingwardWorld) -> Result<()> {
    // Fixed wait: asserting an *absence*, and the tail refreshes 500ms after its
    // signal, so an immediate check would pass with the tail wrongly on.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let driver = world.driver()?;
    driver.expect_visible("pings-empty").await?;
    driver.expect_absent("ping-row").await
}

#[then("the ping filters are hidden")]
async fn ping_filters_hidden(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_hidden("pings-filters").await
}
