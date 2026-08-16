//! The `/ping/*` endpoints other than plain success — a port of
//! `ping_kinds.steps.js`.

use anyhow::{Result, ensure};
use cucumber::{then, when};
use pingward_e2e::PingKind;
use pingward_e2e::actions::read_ping_url;
use pingward_e2e::dom::{Dom, Within, click_when_ready};
use pingward_e2e::world::PingwardWorld;

#[when(expr = "I send an exit code {int} ping")]
async fn send_exit_code_ping(world: &mut PingwardWorld, code: i32) -> Result<()> {
    let ping_url = read_ping_url(world).await?;
    world
        .api()?
        .ping(&ping_url, PingKind::ExitCode(code))
        .await
}

#[when(expr = "I send a {string} ping with body {string}")]
async fn send_ping_with_body(world: &mut PingwardWorld, kind: String, body: String) -> Result<()> {
    // The server records up to `ping::MAX_BODY` of the request body and
    // renders it on the check page.
    let ping_url = read_ping_url(world).await?;
    world
        .api()?
        .ping_with_body(&ping_url, PingKind::parse(&kind)?, &body)
        .await
}

#[when("I ping an unknown UUID")]
async fn ping_unknown_uuid(world: &mut PingwardWorld) -> Result<()> {
    // An unknown uuid 404s without ever reaching a check page, so this drives
    // the request directly and stashes the status for the assertion.
    let url = format!(
        "{}/ping/00000000-0000-0000-0000-000000000000",
        world.base_url()?
    );
    world.ping_status = Some(world.api()?.ping_status(&url, PingKind::Success).await?);
    Ok(())
}

#[then(expr = "the ping response status is {int}")]
async fn ping_response_status(world: &mut PingwardWorld, status: u16) -> Result<()> {
    let seen = world
        .ping_status
        .ok_or_else(|| anyhow::anyhow!("no step recorded a ping response"))?;
    ensure!(seen == status, "the ping answered {seen}, not {status}");
    Ok(())
}

#[then("the ping help documents the fail and start endpoints")]
async fn ping_help_documents_endpoints(world: &mut PingwardWorld) -> Result<()> {
    // The collapsible "How do I ping this check?" card documents every ping
    // signal. Its content is in the document but closed, so open it first.
    let driver = world.driver()?;
    let help = driver.test_id("ping-help").await?;
    let summary = help
        .css_opt("summary")
        .await?
        .ok_or_else(|| anyhow::anyhow!("the ping help has no summary to open"))?;
    click_when_ready(&summary).await?;
    driver.expect_text("ping-help", "/fail").await?;
    driver.expect_text("ping-help", "/start").await
}

/// The recent-pings kind cell renders as `.pill.{class}`; this maps the
/// Gherkin label to that class.
fn pill_class(kind: &str) -> Result<&'static str> {
    Ok(match kind {
        "success" => "ok",
        "fail" => "fail",
        "start" => "start",
        "log" => "log",
        other => anyhow::bail!("no ping pill is called `{other}`"),
    })
}

#[then(expr = "the recent pings table shows a {string} ping")]
async fn pings_table_shows(world: &mut PingwardWorld, kind: String) -> Result<()> {
    // Scoped to `#pings-section`: `.badge` is the status badge at the top of
    // the page, and the "How do I ping" help renders `.pill` in its endpoint
    // legend.
    world
        .driver()?
        .expect_visible_css(&format!("#pings-section .pill.{}", pill_class(&kind)?))
        .await
}

#[then(expr = "the recent pings table shows the exit {string}")]
async fn pings_table_shows_exit(world: &mut PingwardWorld, exit: String) -> Result<()> {
    world.driver()?.expect_exact_text_somewhere(&exit).await
}

#[when("I expand the latest ping row")]
async fn expand_latest_ping_row(world: &mut PingwardWorld) -> Result<()> {
    // Recent pings render newest-first, so the first `tr.toggle` is the row
    // just created. Only rows with a non-empty body render as toggle rows.
    world.driver()?.click_css("tr.toggle").await
}

#[then(expr = "the captured output shows {string}")]
async fn captured_output_shows(world: &mut PingwardWorld, text: String) -> Result<()> {
    // Scoped to the pings section: the ping-help card also renders an `.out`
    // block.
    let driver = world.driver()?;
    driver.expect_visible_css("#pings-section .out").await?;
    driver.expect_text_css("#pings-section .out", &text).await
}
