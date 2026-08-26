//! The check page's opt-in SSE live tail.
//!
//! `live_tail.feature`'s two scenarios are a pair: the "without the live tail"
//! one is the control, proving via its final reload that the ping was recorded,
//! so the live-tail scenario cannot pass on a ping that never registered.

use std::time::Duration;

use anyhow::Result;
use cucumber::{then, when};
use pingward_e2e::dom::Dom;
use pingward_e2e::world::PingwardWorld;

#[when("I turn on the live tail")]
async fn turn_on_live_tail(world: &mut PingwardWorld) -> Result<()> {
    // The backend publishes a "changed" event only while
    // `events.receiver_count() > 0`, and a ping sent before the `EventSource`
    // is open is dropped with no catch-up — hence the wait for
    // `data-live="open"`.
    let driver = world.driver()?;
    driver.click("pings-live").await?;
    driver
        .expect_attr("[data-testid=\"pings-live\"]", "data-live", Some("open"))
        .await
}

#[then("the recent pings table still shows no pings")]
async fn still_no_pings(world: &mut PingwardWorld) -> Result<()> {
    // A fixed wait, because the claim is the *absence* of an update over a
    // window: the live tail refreshes 500ms after its signal, so asserting
    // immediately passes even with the tail wrongly always-on.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let driver = world.driver()?;
    driver.expect_visible("pings-empty").await?;
    driver.expect_absent("ping-row").await
}

#[then("the ping filters are hidden")]
async fn ping_filters_hidden(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_hidden("pings-filters").await
}
