//! The instance settings form, whose inputs have ids but no `data-testid`.

use anyhow::Result;
use cucumber::{then, when};
use pingward_e2e::dom::{Dom, submit_element};
use pingward_e2e::world::PingwardWorld;

#[when(expr = "I fill the settings field {string} with {string}")]
async fn fill_settings_field(
    world: &mut PingwardWorld,
    field: String,
    value: String,
) -> Result<()> {
    world.driver()?.fill_css(&format!("#{field}"), &value).await
}

#[when("I save the settings form")]
async fn save_settings(world: &mut PingwardWorld) -> Result<()> {
    // Success redirects and a validation error re-renders, but the URL is
    // unchanged either way, so checking it would resolve against the stale
    // pre-submit DOM still showing the typed values. `submit_element` waits for
    // the document to be replaced.
    let driver = world.driver()?;
    let button = driver
        .button_named("Save changes")
        .await?
        .ok_or_else(|| anyhow::anyhow!("the settings form has no `Save changes` button"))?;
    submit_element(driver, &button).await
}

#[then(expr = "the settings field {string} shows {string}")]
async fn settings_field_shows(
    world: &mut PingwardWorld,
    field: String,
    value: String,
) -> Result<()> {
    world
        .driver()?
        .expect_value_css(&format!("#{field}"), &value)
        .await
}

#[then(expr = "the settings form shows the error {string}")]
async fn settings_error(world: &mut PingwardWorld, message: String) -> Result<()> {
    // The message quotes the offending name, so the feature file escapes those
    // quotes and the `{string}` capture arrives with the backslashes intact.
    world
        .driver()?
        .expect_exact_text_css(".flash.err", &pingward_e2e::unescape(&message))
        .await
}

#[then(expr = "the settings page shows the flash {string}")]
async fn settings_flash(world: &mut PingwardWorld, message: String) -> Result<()> {
    // A one-shot flash, backed by a cookie cleared on the render that shows it.
    world
        .driver()?
        .expect_exact_text("settings-flash", &message)
        .await
}

#[then("the settings page shows no flash")]
async fn settings_no_flash(world: &mut PingwardWorld) -> Result<()> {
    // A reload, or a rejected save that never sets the cookie, must not show it.
    world.driver()?.expect_absent("settings-flash").await
}
