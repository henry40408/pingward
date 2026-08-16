//! Renaming and re-scheduling existing projects and checks, and the markdown
//! descriptions — a port of `edit_flows.steps.js`.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::dom::{Dom, TextContent, Within, submit_element};
use pingward_e2e::world::PingwardWorld;

/// Follows the page's single uppercase `Edit` link.
///
/// Exact matching is load-bearing: a project with checks also renders a
/// per-row lowercase `edit` link, which a case-insensitive substring match
/// would reach first.
async fn open_edit_form(world: &PingwardWorld, expected: &str) -> Result<()> {
    let driver = world.driver()?;
    let body = driver.find(thirtyfour::By::Tag("body")).await?;
    let link = body
        .link_named_exact("Edit")
        .await?
        .ok_or_else(|| anyhow::anyhow!("the page has no `Edit` link"))?;
    submit_element(driver, &link).await?;
    world.expect_path_matching(expected).await
}

#[given("I open the project edit form")]
#[when("I open the project edit form")]
async fn open_project_edit_form(world: &mut PingwardWorld) -> Result<()> {
    open_edit_form(world, r"/projects/\d+/edit$").await
}

#[when("I open the check edit form")]
async fn open_check_edit_form(world: &mut PingwardWorld) -> Result<()> {
    open_edit_form(world, r"/checks/\d+/edit$").await
}

#[when(expr = "I change the project name to {string}")]
async fn change_project_name(world: &mut PingwardWorld, name: String) -> Result<()> {
    let driver = world.driver()?;
    driver.fill("project-name-input", &name).await?;
    driver.submit("project-submit").await
}

#[when(expr = "I change the check name to {string}")]
async fn change_check_name(world: &mut PingwardWorld, name: String) -> Result<()> {
    let driver = world.driver()?;
    driver.fill("check-name-input", &name).await?;
    driver.submit("check-submit").await
}

#[when(expr = "I change the check period to {int}")]
async fn change_check_period(world: &mut PingwardWorld, period: i64) -> Result<()> {
    let driver = world.driver()?;
    driver
        .fill("check-period-input", &period.to_string())
        .await?;
    driver.submit("check-submit").await
}

#[when(expr = "I change the check grace to {int}")]
async fn change_check_grace(world: &mut PingwardWorld, grace: i64) -> Result<()> {
    let driver = world.driver()?;
    driver.fill_css("#grace_secs", &grace.to_string()).await?;
    driver.submit("check-submit").await
}

#[when(expr = "I change the check timezone to {string}")]
async fn change_check_timezone(world: &mut PingwardWorld, timezone: String) -> Result<()> {
    let driver = world.driver()?;
    driver.fill_css("#timezone", &timezone).await?;
    driver.submit("check-submit").await
}

#[then(expr = "the check name is {string}")]
async fn check_name_is(world: &mut PingwardWorld, name: String) -> Result<()> {
    let heading = world.driver()?.heading_opt(&name).await?;
    ensure!(heading.is_some(), "no heading reads {name:?}");
    Ok(())
}

#[then(expr = "the check timezone field shows {string}")]
async fn timezone_field_shows(world: &mut PingwardWorld, timezone: String) -> Result<()> {
    // The check page has no timezone display, so persistence is verified by
    // reopening the edit form and reading the pre-filled value back out.
    world.driver()?.expect_value_css("#timezone", &timezone).await
}

#[then(expr = "the check period field shows {string}")]
async fn period_field_shows(world: &mut PingwardWorld, period: String) -> Result<()> {
    world
        .driver()?
        .expect_value("check-period-input", &period)
        .await
}

#[given(expr = "I set the project description to {string}")]
#[when(expr = "I set the project description to {string}")]
async fn set_project_description(world: &mut PingwardWorld, description: String) -> Result<()> {
    let driver = world.driver()?;
    driver
        .fill("project-description-input", &description)
        .await?;
    driver.submit("project-submit").await
}

#[when(expr = "I set the check description to {string}")]
async fn set_check_description(world: &mut PingwardWorld, description: String) -> Result<()> {
    let driver = world.driver()?;
    driver.fill("check-description-input", &description).await?;
    driver.submit("check-submit").await
}

#[then(expr = "the project description shows {string} in bold")]
async fn project_description_bold(world: &mut PingwardWorld, text: String) -> Result<()> {
    description_bold(world, "project-description", &text).await
}

#[then(expr = "the check description shows {string} in bold")]
async fn check_description_bold(world: &mut PingwardWorld, text: String) -> Result<()> {
    description_bold(world, "check-description", &text).await
}

/// The description card renders through `markdown.rs` (escape first, then a
/// small tag whitelist), so `**bold**` becomes a real `<strong>` element
/// rather than escaped markers.
async fn description_bold(world: &PingwardWorld, id: &str, text: &str) -> Result<()> {
    let card = world.driver()?.test_id(id).await?;
    let strong = card
        .css_opt("strong")
        .await?
        .ok_or_else(|| anyhow::anyhow!("`{id}` rendered no <strong>"))?;
    let rendered = strong.normalized_text().await?;
    ensure!(rendered == text, "the bold run reads {rendered:?}, not {text:?}");
    Ok(())
}

#[then("the check row shows a truncated description")]
async fn truncated_description(world: &mut PingwardWorld) -> Result<()> {
    // The project page's check row shows `markdown::truncate_plain` — markers
    // stripped, no tags, capped at 120 characters with a trailing ellipsis.
    // Asserting the ellipsis is what proves truncation happened, rather than
    // the whole description simply fitting.
    let driver = world.driver()?;
    driver.expect_visible("check-description-summary").await?;
    driver
        .expect_text("check-description-summary", "…")
        .await?;
    let summary = driver.text_of("check-description-summary").await?;
    ensure!(
        !summary.contains("**"),
        "the summary still carries markdown markers: {summary:?}"
    );
    Ok(())
}
