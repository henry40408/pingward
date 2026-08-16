//! The check page's opt-in SSE live tail — a port of `live_tail.steps.js`.
//!
//! The two scenarios in `live_tail.feature` are a deliberate pair: the
//! "without the live tail" scenario is the control, proving via its final
//! reload that the ping really was recorded — so the live-tail scenario
//! passing can only be attributed to the live tail itself, never to a ping
//! that silently failed to register.

use std::time::Duration;

use anyhow::Result;
use cucumber::{then, when};
use pingward_e2e::dom::Dom;
use pingward_e2e::world::PingwardWorld;

#[when("I turn on the live tail")]
async fn turn_on_live_tail(world: &mut PingwardWorld) -> Result<()> {
    // The backend publishes an SSE "changed" event only while
    // `events.receiver_count() > 0` (see ARCHITECTURE.md's live-tail signal
    // bus). A ping sent before the `EventSource` is actually open is dropped
    // with no later signal to catch up on, so this must wait for
    // `data-live="open"` — without it, sending the ping straight after the
    // click is racy.
    let driver = world.driver()?;
    driver.click("pings-live").await?;
    driver
        .expect_attr("[data-testid=\"pings-live\"]", "data-live", Some("open"))
        .await
}

#[then("the recent pings table still shows no pings")]
async fn still_no_pings(world: &mut PingwardWorld) -> Result<()> {
    // A fixed wait is normally an anti-pattern, but proving the *absence* of
    // an update over a window is exactly where it is the right tool: the live
    // tail refreshes 500ms after its SSE signal, so asserting immediately
    // would pass even if the live tail were wrongly always-on (verified — it
    // did). Wait past the debounce plus a fragment fetch, then assert.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let driver = world.driver()?;
    driver.expect_visible("pings-empty").await?;
    driver.expect_absent("ping-row").await
}

#[then("the ping filters are hidden")]
async fn ping_filters_hidden(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_hidden("pings-filters").await
}
