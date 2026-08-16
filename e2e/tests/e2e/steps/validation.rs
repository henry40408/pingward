//! Server-side validation of the project and check forms — a port of
//! `validation.steps.js`.
//!
//! The fields these steps drive carry an `id` but no `data-testid`
//! (`#scan_interval_secs`, `#max_runtime_secs`, `#timezone`), so they are
//! addressed by selector exactly as the JavaScript suite addressed them.

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
    // A text input with a `<datalist>`, so filling it works exactly as it
    // would for a plain field.
    world.driver()?.fill_css("#timezone", &value).await
}

#[then("the timezone field offers a list of zones")]
async fn timezone_offers_zones(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver
        .expect_attr("#timezone", "list", Some("tz-list"))
        .await?;
    // Non-vacuity guard: an empty `<datalist>` would satisfy "the field
    // accepts a zone" on its own.
    let count = driver.count_css("#tz-list option").await?;
    ensure!(count > 100, "the zone list offers only {count} options");
    driver
        .expect_count(r#"#tz-list option[value="Asia/Taipei"]"#, 1)
        .await
}
