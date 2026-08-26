//! Server-side validation of the project and check forms.
//!
//! The fields these steps drive carry an `id` but no `data-testid`, so they are
//! addressed by selector.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::dom::Dom;
use pingward_e2e::world::PingwardWorld;

#[given("I open the new project form")]
async fn open_new_project_form(world: &mut PingwardWorld) -> Result<()> {
    world.goto("/projects/new").await
}

#[when(expr = "I fill the project name with {string}")]
async fn fill_project_name(world: &mut PingwardWorld, name: String) -> Result<()> {
    world.driver()?.fill("project-name-input", &name).await
}

#[when(expr = "I fill the project scan interval with {string}")]
async fn fill_scan_interval(world: &mut PingwardWorld, value: String) -> Result<()> {
    world
        .driver()?
        .fill_css("#scan_interval_secs", &value)
        .await
}

#[when("I submit the project form")]
async fn submit_project_form(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.submit("project-submit").await
}

#[then(expr = "the project form shows the error {string}")]
async fn project_form_error(world: &mut PingwardWorld, message: String) -> Result<()> {
    world
        .driver()?
        .expect_exact_text_css(".flash.err", &message)
        .await
}

#[then(expr = "the project name field shows {string}")]
async fn project_name_shows(world: &mut PingwardWorld, name: String) -> Result<()> {
    world
        .driver()?
        .expect_value("project-name-input", &name)
        .await
}

#[when(expr = "I fill the check max runtime with {string}")]
async fn fill_max_runtime(world: &mut PingwardWorld, value: String) -> Result<()> {
    world.driver()?.fill_css("#max_runtime_secs", &value).await
}

#[when(expr = "I fill the check timezone with {string}")]
async fn fill_timezone(world: &mut PingwardWorld, value: String) -> Result<()> {
    // A text input with a `<datalist>`, so it fills like a plain field.
    world.driver()?.fill_css("#timezone", &value).await
}

#[then("the timezone field offers a list of zones")]
async fn timezone_offers_zones(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver
        .expect_attr("#timezone", "list", Some("tz-list"))
        .await?;
    // Non-vacuity guard: an empty `<datalist>` would satisfy the above alone.
    let count = driver.count_css("#tz-list option").await?;
    ensure!(count > 100, "the zone list offers only {count} options");
    driver
        .expect_count(r#"#tz-list option[value="Asia/Taipei"]"#, 1)
        .await
}

#[then("every check duration field offers the same list of durations")]
async fn duration_fields_offer_suggestions(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    // Every duration field points at the same list id, so the five cannot drift
    // into five vocabularies.
    for field in [
        "#period_secs",
        "#grace_secs",
        "#scan_interval_secs",
        "#max_runtime_secs",
        "#nag_interval_secs",
    ] {
        driver.expect_attr(field, "list", Some("dur-list")).await?;
    }
    // Non-vacuity guard, as for the zone list.
    let count = driver.count_css("#dur-list option").await?;
    ensure!(count > 3, "the duration list offers only {count} options");
    driver
        .expect_count(r#"#dur-list option[value="5m"]"#, 1)
        .await
}
