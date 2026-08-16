//! The instance settings form — a port of `settings.steps.js`.
//!
//! The settings inputs have ids but no `data-testid`, so they are driven by
//! id.

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
    // Saving POSTs to `/admin`. On success the handler 303-redirects back; on
    // a validation error it re-renders inline. Either way the URL is
    // unchanged, so an assertion that merely checked it would resolve against
    // the stale pre-submit DOM — a false pass, since the just-typed values are
    // still shown there. `submit_element` waits for the document to be
    // replaced, which ties the step to the reloaded page.
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
    // The unknown-timezone message quotes the offending name, so the feature
    // file escapes those quotes — see `pingward_e2e::unescape`.
    world
        .driver()?
        .expect_exact_text_css(".flash.err", &pingward_e2e::unescape(&message))
        .await
}

#[then(expr = "the settings page shows the flash {string}")]
async fn settings_flash(world: &mut PingwardWorld, message: String) -> Result<()> {
    // A one-shot success flash, backed by a flash cookie cleared on this
    // render — the same mechanism as the check page's notify-channels flash.
    world
        .driver()?
        .expect_exact_text("settings-flash", &message)
        .await
}

#[then("the settings page shows no flash")]
async fn settings_no_flash(world: &mut PingwardWorld) -> Result<()> {
    // One-shot means exactly that: a fresh render — a reload, or a rejected
    // save that re-renders without ever setting the cookie — must not show it.
    world.driver()?.expect_absent("settings-flash").await
}
