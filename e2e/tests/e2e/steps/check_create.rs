//! The check-creation form's branches — a port of `check_create.steps.js`.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::dom::Dom;
use pingward_e2e::world::PingwardWorld;

/// Opens the new-check form from the current project page.
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
    // Cron mode: pick the kind and supply a 6-field expression. `period_secs`
    // is left blank, since it is ignored in cron mode. On success the handler
    // redirects to the check page.
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

/// The human-readable duration form (`1h30m`), as opposed to the bare-integer
/// variant above.
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
    // The schedule label renders on the check page; for a cron check it is the
    // raw expression.
    world.driver()?.expect_text_somewhere(&text).await
}

#[then("I am still on the new check form")]
async fn still_on_new_check_form(world: &mut PingwardWorld) -> Result<()> {
    // Submitting with an empty name is blocked client-side by the input's
    // `required` attribute, so no POST fires and the form stays put.
    world
        .expect_path_matching(r"/projects/\d+/checks/new$")
        .await?;
    world.driver()?.expect_visible("check-submit").await
}

#[then("only the period field is shown")]
async fn only_period_shown(world: &mut PingwardWorld) -> Result<()> {
    // The kind select drives `:has()` rules in `app.css` — not script — which
    // hide the field belonging to the other kind, so the two are never visible
    // at once.
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
    // The unknown-timezone message quotes the offending name, so the feature
    // file escapes those quotes — see `pingward_e2e::unescape`.
    world
        .driver()?
        .expect_exact_text_css(".flash.err", &pingward_e2e::unescape(&message))
        .await
}
