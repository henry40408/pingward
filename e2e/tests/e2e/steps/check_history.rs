//! The check page's pings pager, filters and heartbeat strip.

use anyhow::{Result, ensure};
use cucumber::{then, when};
use pingward_e2e::PingKind;
use pingward_e2e::actions::read_ping_url;
use pingward_e2e::dom::Dom;
use pingward_e2e::world::PingwardWorld;

#[when(expr = "I send {int} {string} pings")]
async fn send_many_pings(world: &mut PingwardWorld, count: usize, kind: String) -> Result<()> {
    // Each is a bare GET with no body, so each renders as a single plain
    // (non-toggle) `tr`.
    let ping_url = read_ping_url(world).await?;
    let api = world.api()?;
    let kind = PingKind::parse(&kind)?;
    for _ in 0..count {
        api.ping(&ping_url, kind).await?;
    }
    Ok(())
}

#[then(expr = "the pings table shows {int} rows")]
async fn pings_table_rows(world: &mut PingwardWorld, count: usize) -> Result<()> {
    world
        .driver()?
        .expect_count("[data-testid=\"ping-row\"]", count)
        .await
}

#[then(expr = "the pings {word} link is enabled")]
async fn pager_link_enabled(world: &mut PingwardWorld, direction: String) -> Result<()> {
    pager_link_state(world, &direction, false).await
}

#[then(expr = "the pings {word} link is disabled")]
async fn pager_link_disabled(world: &mut PingwardWorld, direction: String) -> Result<()> {
    pager_link_state(world, &direction, true).await
}

/// The pager's ends are always shown: reaching one renders a muted
/// `<span class="btn disabled">` rather than hiding the control, so the
/// assertion is on the class, not on visibility.
async fn pager_link_state(
    world: &PingwardWorld,
    direction: &str,
    expected_disabled: bool,
) -> Result<()> {
    let driver = world.driver()?;
    let id = format!("pings-{direction}");
    let link = driver.test_id(&id).await?;
    let classes = link.attr("class").await?.unwrap_or_default();
    let disabled = classes.split_whitespace().any(|class| class == "disabled");
    ensure!(
        disabled == expected_disabled,
        "the {direction} link's classes are {classes:?}"
    );
    Ok(())
}

#[when("I click the pings older link")]
async fn click_older(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.click("pings-older").await
}

#[when("I click the pings newer link")]
async fn click_newer(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.click("pings-newer").await
}

// Not `no_js.feature`'s near-identically worded "I filter *the* pings by
// kind": with script the click is cancelled and the section swapped in place,
// so there is no navigation to wait for. Keeping the two apart stops one
// waiting strategy being used for the other path.
#[when(expr = "I filter pings by kind {string}")]
async fn filter_pings_by_kind(world: &mut PingwardWorld, kind: String) -> Result<()> {
    // The row-count assertion that follows waits for the fragment swap.
    let driver = world.driver()?;
    driver.select_option("pings-kind", &kind).await?;
    driver.click("pings-apply").await
}

#[when("I clear the pings filter")]
async fn clear_pings_filter(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.click("pings-clear").await
}

#[when(expr = "I set the pings from date to {string}")]
async fn set_pings_from(world: &mut PingwardWorld, value: String) -> Result<()> {
    world.driver()?.fill("pings-from", &value).await
}

#[when("I apply the pings filter")]
async fn apply_pings_filter(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.click("pings-apply").await
}

#[then(expr = "the pings from date is {string}")]
async fn pings_from_is(world: &mut PingwardWorld, value: String) -> Result<()> {
    // The local wall-clock value round-trips through UTC, so it matches what
    // was entered whatever the runner's time zone.
    world.driver()?.expect_value("pings-from", &value).await
}

#[then("the pings clear filter link is visible")]
async fn pings_clear_visible(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible("pings-clear").await
}

#[then("the pings clear filter link is not visible")]
async fn pings_clear_absent(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_absent("pings-clear").await
}

#[then("the newest heartbeat bar is flush with the strip's right edge")]
async fn newest_bar_flush_right(world: &mut PingwardWorld) -> Result<()> {
    // A CSS-only invariant: the bars are all rendered either way, and this
    // checks which of them the clipping box lets through.
    let gap = world
        .driver()?
        .eval(
            "const beat = document.querySelector('.beat');\
             const bars = beat.querySelectorAll('i');\
             const last = bars[bars.length - 1].getBoundingClientRect();\
             return Math.round(beat.getBoundingClientRect().right - last.right);",
        )
        .await?
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("the edge probe did not return a number"))?;
    ensure!(
        gap <= 1.0,
        "the newest run sits {gap}px from the right edge — the strip is not right-aligned"
    );
    Ok(())
}

#[then("the oldest heartbeat bars are clipped off the left")]
async fn oldest_bars_clipped(world: &mut PingwardWorld) -> Result<()> {
    // `scrollWidth` does not count overflow off the *left* edge, so this
    // compares what was rendered against what the clipping box lets through.
    let measured = world
        .driver()?
        .eval(
            "const beat = document.querySelector('.beat');\
             const box = beat.getBoundingClientRect();\
             const bars = [...beat.querySelectorAll('i')];\
             return {\
               rendered: bars.length,\
               visible: bars.filter((b) => b.getBoundingClientRect().left >= box.left - 0.5).length,\
               box: Math.round(box.width),\
             };",
        )
        .await?;
    let field = |name: &str| {
        measured
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default()
    };
    let (rendered, visible, box_width) = (field("rendered"), field("visible"), field("box"));
    ensure!(rendered > 30, "only {rendered} bars were rendered");
    ensure!(
        visible < rendered,
        "all {rendered} bars fit the {box_width}px strip — nothing is being clipped, \
         so this proves nothing"
    );
    Ok(())
}
