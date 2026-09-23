//! The check-creation form's branches.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::dom::Dom;
use pingward_e2e::world::PingwardWorld;

async fn open_new_check_form(world: &PingwardWorld) -> Result<()> {
    world.driver()?.submit("new-check-link").await?;
    world
        .expect_path_matching(r"/projects/\d+/checks/new$")
        .await
}

#[given("I open the new check form")]
async fn given_new_check_form(world: &mut PingwardWorld) -> Result<()> {
    open_new_check_form(world).await
}

#[when(expr = "I create a cron check named {string} with expression {string}")]
async fn create_cron_check(world: &mut PingwardWorld, name: String, expr: String) -> Result<()> {
    open_new_check_form(world).await?;
    let driver = world.driver()?;
    driver.fill("check-name-input", &name).await?;
    driver.select_option_css("#schedule_kind", "cron").await?;
    driver.fill_css("#cron_expr", &expr).await?;
    driver.submit("check-submit").await?;
    world.expect_path_matching(r"/checks/\d+$").await
}

#[when(expr = "I fill the check name with {string}")]
async fn fill_check_name(world: &mut PingwardWorld, name: String) -> Result<()> {
    world.driver()?.fill("check-name-input", &name).await
}

#[when(expr = "I fill the check period with {int}")]
async fn fill_check_period_int(world: &mut PingwardWorld, period: i64) -> Result<()> {
    world
        .driver()?
        .fill("check-period-input", &period.to_string())
        .await
}

/// Human-readable durations (`1h30m`).
#[when(expr = "I fill the check period with {string}")]
async fn fill_check_period_text(world: &mut PingwardWorld, period: String) -> Result<()> {
    world.driver()?.fill("check-period-input", &period).await
}

#[when(expr = "I choose the {string} schedule kind")]
async fn choose_schedule_kind(world: &mut PingwardWorld, kind: String) -> Result<()> {
    world
        .driver()?
        .select_option_css("#schedule_kind", &kind)
        .await
}

#[when("I submit the check form")]
async fn submit_check_form(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.click("check-submit").await
}

#[then(expr = "the check schedule shows {string}")]
async fn schedule_shows(world: &mut PingwardWorld, text: String) -> Result<()> {
    world.driver()?.expect_text_somewhere(&text).await
}

#[then("I am still on the new check form")]
async fn still_on_new_check_form(world: &mut PingwardWorld) -> Result<()> {
    // `required` blocks the submit client-side; no POST fires.
    world
        .expect_path_matching(r"/projects/\d+/checks/new$")
        .await?;
    world.driver()?.expect_visible("check-submit").await
}

#[then("only the period field is shown")]
async fn only_period_shown(world: &mut PingwardWorld) -> Result<()> {
    // Switched by `:has()` rules in `app.css`, not script.
    let driver = world.driver()?;
    driver.expect_visible("check-period-input").await?;
    driver.expect_hidden_css("#cron_expr").await
}

#[then("only the cron field is shown")]
async fn only_cron_shown(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_visible_css("#cron_expr").await?;
    driver.expect_hidden("check-period-input").await
}

#[then("the check name field is required")]
async fn name_field_required(world: &mut PingwardWorld) -> Result<()> {
    let missing = world
        .driver()?
        .eval(
            "return document.querySelector('[data-testid=\"check-name-input\"]')\
             .validity.valueMissing;",
        )
        .await?;
    ensure!(
        missing.as_bool() == Some(true),
        "the browser did not refuse the empty name (validity.valueMissing was {missing})"
    );
    Ok(())
}

#[then(expr = "the check form shows the error {string}")]
async fn check_form_error(world: &mut PingwardWorld, message: String) -> Result<()> {
    // The message contains quotes; `{string}` keeps their escaping backslashes.
    world
        .driver()?
        .expect_exact_text_css(".flash.err", &pingward_e2e::unescape(&message))
        .await
}
