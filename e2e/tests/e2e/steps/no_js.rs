//! Steps for `no_js.feature`, whose `@nojs` tag opens its sessions with the
//! page's own scripts disabled. Nothing here may reach for a `data-testid`
//! that only exists after `app.js` has run — that is the whole point of the
//! file.
//!
//! The two assertions about the ping-output panel are also used from
//! `check_history.feature` with script *on*, where they assert the opposite: a
//! fix that simply left every panel open would satisfy the no-JS scenario and
//! break nothing else, so the collapse is pinned from both sides.

use anyhow::{Result, ensure};
use cucumber::{then, when};
use pingward_e2e::PingKind;
use pingward_e2e::actions::read_ping_url;
use pingward_e2e::dom::{Dom, Within, click_when_ready};
use pingward_e2e::world::PingwardWorld;

#[when(expr = "I send a failing ping with output {string}")]
async fn send_failing_ping_with_output(world: &mut PingwardWorld, output: String) -> Result<()> {
    // A finish ping carrying a body, which is what pingward captures and
    // renders in the expandable panel. POST, because that is how a body
    // reaches it.
    let ping_url = read_ping_url(world).await?;
    world
        .api()?
        .ping_with_body(&ping_url, PingKind::Fail, &output)
        .await
}

#[then(expr = "the captured output {string} is visible")]
async fn captured_output_visible(world: &mut PingwardWorld, output: String) -> Result<()> {
    // Visibility rather than a text match: the output has always been *in* the
    // DOM, hidden by `tr.exp { display: none }`. Being rendered is the claim.
    captured_output_displayed(world, &output, true).await
}

#[then(expr = "the captured output {string} is hidden")]
async fn captured_output_hidden(world: &mut PingwardWorld, output: String) -> Result<()> {
    captured_output_displayed(world, &output, false).await
}

async fn captured_output_displayed(
    world: &PingwardWorld,
    output: &str,
    expected: bool,
) -> Result<()> {
    let driver = world.driver()?;
    pingward_e2e::wait::eventually(
        &format!(
            "the captured output is {}",
            if expected { "shown" } else { "hidden" }
        ),
        || async {
            for panel in driver.css_with_text("#pings-section .out", output).await? {
                if panel.is_displayed().await.unwrap_or(false) == expected {
                    return Ok(true);
                }
            }
            Ok(false)
        },
    )
    .await
}

#[then("the expand carets are invisible")]
async fn carets_invisible(world: &mut PingwardWorld) -> Result<()> {
    // Carets are drawn with `opacity`, not `display`, so the column keeps its
    // width — which makes "invisible" a computed-style assertion rather than a
    // visibility one.
    let carets = world.driver()?.css_all("#pings-section .caret").await?;
    ensure!(
        !carets.is_empty(),
        "no carets on the page — assertion is vacuous"
    );
    for caret in carets {
        let opacity = caret.css_value("opacity").await?;
        ensure!(
            opacity.parse::<f64>().unwrap_or(1.0) == 0.0,
            "a caret renders at opacity {opacity}"
        );
    }
    Ok(())
}

#[when("I expand the first ping row")]
async fn expand_first_ping_row(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.click_css("#pings-section tr.toggle").await
}

#[when(expr = "I click the dashboard check link for {string}")]
async fn click_dashboard_check_link(world: &mut PingwardWorld, name: String) -> Result<()> {
    // The row's real anchor, not the delegated `data-href` handler.
    let driver = world.driver()?;
    let row = driver
        .test_ids_with_text("dashboard-check-row", &name)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no dashboard row for {name:?}"))?;
    let link = row
        .link_named(&name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the row for {name:?} has no name link"))?;
    click_when_ready(&link).await
}

// A plain form submission: pick the value, press the button, follow the
// navigation. With script the same click is cancelled and the section swapped
// in place, which is why `check_history.rs` keeps its own near-identically
// worded step rather than sharing this one — waiting for a navigation that
// never comes would hang there, and *not* waiting for one here reads the
// pre-submit page.
#[when(expr = "I filter the pings by kind {string}")]
async fn filter_pings_by_kind_unscripted(world: &mut PingwardWorld, kind: String) -> Result<()> {
    let driver = world.driver()?;
    driver.select_option("pings-kind", &kind).await?;
    driver.submit("pings-apply").await
}

#[when(expr = "I filter the notifications by event {string}")]
async fn filter_notifications_by_event(world: &mut PingwardWorld, event: String) -> Result<()> {
    let driver = world.driver()?;
    driver.select_option("notifs-event", &event).await?;
    driver.submit("notifs-apply").await
}

#[then(expr = "the pings kind filter shows {string}")]
async fn pings_kind_filter_shows(world: &mut PingwardWorld, kind: String) -> Result<()> {
    // The selected value surviving a round trip is what proves the filter
    // reached the server and came back rendered, rather than the page merely
    // reloading.
    world.driver()?.expect_value("pings-kind", &kind).await
}

#[then(expr = "the notifications event filter shows {string}")]
async fn notifs_event_filter_shows(world: &mut PingwardWorld, event: String) -> Result<()> {
    world.driver()?.expect_value("notifs-event", &event).await
}

#[when(expr = "my system prefers {string}")]
async fn system_prefers(world: &mut PingwardWorld, scheme: String) -> Result<()> {
    // A CDP-level media override, so it works with scripting disabled — the
    // page never has to be asked anything.
    world.browser()?.emulate_color_scheme(&scheme).await
}

/// The body background's relative luminance.
///
/// Asserted by brightness rather than an exact token value: the claim is "this
/// is a light page", and pinning `#f3f5f8` would turn any future palette tweak
/// into a failure that says nothing about whether the theme still works.
async fn background_luminance(world: &PingwardWorld) -> Result<f64> {
    world
        .driver()?
        .eval(
            "const [r, g, b] = getComputedStyle(document.body)\
               .backgroundColor.match(/\\d+(\\.\\d+)?/g).map(Number);\
             return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;",
        )
        .await?
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("the luminance probe did not return a number"))
}

#[then("the page background is light")]
async fn background_is_light(world: &mut PingwardWorld) -> Result<()> {
    let luminance = background_luminance(world).await?;
    ensure!(
        luminance > 0.5,
        "the background's luminance is {luminance:.3}"
    );
    Ok(())
}

#[then("the page background is dark")]
async fn background_is_dark(world: &mut PingwardWorld) -> Result<()> {
    let luminance = background_luminance(world).await?;
    ensure!(
        luminance < 0.5,
        "the background's luminance is {luminance:.3}"
    );
    Ok(())
}

#[then("the copy button is absent")]
async fn copy_button_absent(world: &mut PingwardWorld) -> Result<()> {
    // Hidden, not gone: `:root:not(.js)` hides these by CSS, so the elements
    // are still in the DOM and a count assertion would fail for the wrong
    // reason — and would keep failing even if the rule worked.
    world.driver()?.expect_hidden_css(".copy").await
}

#[then("the live tail toggle is absent")]
async fn live_toggle_absent(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_hidden("pings-live").await
}

#[then("the theme toggle is absent")]
async fn theme_toggle_absent(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_hidden_css("#pw-theme-toggle").await
}

#[then("the scheduler heartbeat shows an age")]
async fn heartbeat_shows_age(world: &mut PingwardWorld) -> Result<()> {
    let text = world
        .driver()?
        .text_of_css("[data-testid=sched-scan] .hb-ago")
        .await?
        .unwrap_or_default();
    let looks_relative = regex::Regex::new(r"\d+[smhd] ago")
        .expect("the age pattern compiles")
        .is_match(&text);
    ensure!(looks_relative, "the heartbeat age reads {text:?}");
    Ok(())
}

#[when("I start creating a check")]
async fn start_creating_check(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.submit("new-check-link").await?;
    driver.expect_visible("check-name-input").await
}

// "I choose the {string} schedule kind" is shared with `check_create.rs` — a
// plain `<select>` change either way. With no script behind it, `:checked`
// moves and the `:has()` rules in `app.css` re-evaluate on their own.

#[then("the period field is visible")]
async fn period_visible(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible("check-period-input").await
}

#[then("the period field is hidden")]
async fn period_hidden(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_hidden("check-period-input").await
}

#[then("the cron field is visible")]
async fn cron_visible(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible_css("#cron_expr").await
}

#[then("the cron field is hidden")]
async fn cron_hidden(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_hidden_css("#cron_expr").await
}

#[when("I click the delete check button")]
async fn click_delete_check(world: &mut PingwardWorld) -> Result<()> {
    // Deliberately not monitoring's "I delete the check": that one answers a
    // `confirm()` and expects to land back on the project page, which is the
    // scripted path. Here the click is expected to reach an interstitial.
    let driver = world.driver()?;
    let button = driver.test_id("delete-check-button").await?;
    click_when_ready(&button).await
}

#[then("the confirmation page asks about deleting")]
async fn confirmation_page_asks(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_text("confirm-message", "history").await?;
    driver.expect_visible("confirm-submit").await
}

#[when("I confirm the pending action")]
async fn confirm_pending_action(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.submit("confirm-submit").await
}
