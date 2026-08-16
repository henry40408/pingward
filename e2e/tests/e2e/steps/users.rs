//! The `/admin` user table — a port of `users.steps.js`.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::dom::{
    Dom, TextContent, Within, click_when_ready, submit_element, submit_element_confirming,
};
use pingward_e2e::world::PingwardWorld;
use thirtyfour::WebElement;

/// A user's `<tr>`.
///
/// Every control (reset / toggle-admin / toggle-disabled / delete) and status
/// pill carries a row-local `data-testid`, so scoping to the row keeps
/// selectors unambiguous even when a username collides with another row's
/// role-pill text ("member").
async fn user_row(world: &PingwardWorld, username: &str) -> Result<WebElement> {
    world.driver()?.test_id(&format!("user-row-{username}")).await
}

/// An element's tag name.
async fn tag_name(world: &PingwardWorld, element: &WebElement) -> Result<String> {
    Ok(world
        .driver()?
        .execute("return arguments[0].tagName;", vec![element.to_json()?])
        .await?
        .json()
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

/// Drives one of a row's mutating controls and waits for the redirect back to
/// `/admin`.
///
/// Every such control POSTs a form that redirects to `/admin`. Because the URL
/// is unchanged, an assertion that merely checked it would resolve instantly
/// against the stale pre-navigation DOM — a false pass for the "state
/// unchanged" guard scenarios, and a race with the next step's navigation
/// aborting an in-flight POST.
///
/// On the signed-in admin's own row the demote / disable / delete controls
/// render as an inert `<span>` rather than a submitting `<form>` (see the
/// "inert on your own row" scenario), so there is nothing to click. In that
/// case the handler is driven directly with a POST carrying the page's CSRF
/// token, which still proves the handler's own self-guard refuses the action
/// independently of the UI hiding the control. The base path is read off the
/// row's always-present, always-live password-reset form
/// (`/admin/users/{id}/password`), which every other per-row action shares.
async fn submit_row_action(
    world: &PingwardWorld,
    row: &WebElement,
    control: &WebElement,
    action: &str,
) -> Result<()> {
    if tag_name(world, control).await? == "SPAN" {
        let reset = row
            .css_opt(r#"form[action$="/password"]"#)
            .await?
            .ok_or_else(|| anyhow::anyhow!("the row has no password-reset form to read"))?
            .attr("action")
            .await?
            .unwrap_or_default();
        let base = reset.trim_end_matches("/password").to_owned();
        let csrf = world
            .driver()?
            .css_opt(r#"input[name="_csrf"]"#)
            .await?
            .ok_or_else(|| anyhow::anyhow!("the page renders no CSRF field"))?
            .prop("value")
            .await?
            .unwrap_or_default();
        // The handler refuses with a 303 back to /admin. Asserting that
        // explicitly matters: without it a 403 from `csrf_guard` (or an auth
        // bounce) would leave the state unchanged too, and the scenario would
        // pass without ever reaching the self-guard it exists to test.
        let status = world
            .post_form_as_user(&format!("{base}/{action}"), &[("_csrf", csrf.as_str())])
            .await?;
        ensure!(status == 303, "the self-targeted {action} answered {status}");
        world.goto("/admin").await?;
        return Ok(());
    }
    // Destructive controls — delete always, revoke-admin and disable when they
    // would actually change state — raise a `confirm()` first, while "make
    // admin" and "enable" never do. `submit_element_confirming` covers both.
    submit_element_confirming(world.driver()?, control).await
}

/// Fills the "Add user" form and submits it.
///
/// With `admin` set the is_admin checkbox is ticked, so the created account is
/// an admin. The new row's visibility is awaited, so the step only returns once
/// the user has actually rendered.
async fn add_user(
    world: &PingwardWorld,
    username: &str,
    password: &str,
    admin: bool,
) -> Result<()> {
    let driver = world.driver()?;
    driver.fill("user-username-input", username).await?;
    driver.fill("user-password-input", password).await?;
    if admin {
        driver.check("user-admin-checkbox").await?;
    }
    driver.submit("user-submit").await?;
    world.expect_path("/admin").await?;
    world
        .driver()?
        .expect_visible(&format!("user-row-{username}"))
        .await
}

#[given("I am on the users page")]
async fn on_users_page(world: &mut PingwardWorld) -> Result<()> {
    world.goto("/admin").await
}

#[when(expr = "I add a user {string} with password {string}")]
#[given(expr = "a member {string} with password {string} exists")]
async fn add_member(world: &mut PingwardWorld, username: String, password: String) -> Result<()> {
    add_user(world, &username, &password, false).await
}

#[when(expr = "I add an admin user {string} with password {string}")]
#[given(expr = "an admin user {string} with password {string} exists")]
async fn add_admin(world: &mut PingwardWorld, username: String, password: String) -> Result<()> {
    add_user(world, &username, &password, true).await
}

#[when(expr = "I toggle admin on {string}")]
async fn toggle_admin(world: &mut PingwardWorld, username: String) -> Result<()> {
    row_action(world, &username, "user-toggle-admin", "admin").await
}

#[given(expr = "I disable {string}")]
#[when(expr = "I disable {string}")]
#[when(expr = "I enable {string}")]
async fn toggle_disabled(world: &mut PingwardWorld, username: String) -> Result<()> {
    // Enable and disable are the same control: it posts a toggle, and which
    // direction it moves is a property of the row's current state.
    row_action(world, &username, "user-toggle-disabled", "disabled").await
}

#[when(expr = "I delete the user {string}")]
async fn delete_user(world: &mut PingwardWorld, username: String) -> Result<()> {
    row_action(world, &username, "user-delete", "delete").await
}

async fn row_action(
    world: &PingwardWorld,
    username: &str,
    control: &str,
    action: &str,
) -> Result<()> {
    let row = user_row(world, username).await?;
    let control = row.test_id(control).await?;
    submit_row_action(world, &row, &control, action).await?;
    world.expect_path("/admin").await
}

#[when(expr = "I reset {string}'s password to {string}")]
#[when(expr = "I try to reset {string}'s password to {string}")]
async fn reset_password(
    world: &mut PingwardWorld,
    username: String,
    password: String,
) -> Result<()> {
    // The refusal case shares this step body: the handler re-renders `/admin`
    // with an error instead of redirecting, and either way the document is
    // replaced, which is what `submit_element` waits for.
    let row = user_row(world, &username).await?;
    let field = row.test_id("user-reset-input").await?;
    field.clear().await?;
    field.send_keys(&password).await?;
    let submit = row.test_id("user-reset-submit").await?;
    submit_element(world.driver()?, &submit).await
}

#[when(expr = "I attempt to delete {string} but dismiss the confirmation")]
async fn dismiss_delete_confirmation(world: &mut PingwardWorld, username: String) -> Result<()> {
    // Deliberately answers the prompt no, so the form never submits. Reading
    // the message first is what proves the confirmation actually appeared —
    // a missing dialog would leave the row standing too.
    let row = user_row(world, &username).await?;
    let control = row.test_id("user-delete").await?;
    click_when_ready(&control).await?;
    let driver = world.driver()?;
    let message = driver.confirm_message().await?;
    driver.dismiss_confirm().await?;
    ensure!(
        message == "Delete this user? This cannot be undone.",
        "the confirmation read {message:?}"
    );
    Ok(())
}

/// The Gherkin action name, and the `data-testid` of its per-row control.
fn self_row_test_id(action: &str) -> Result<&'static str> {
    Ok(match action {
        "demote" => "user-toggle-admin",
        "disable" => "user-toggle-disabled",
        "delete" => "user-delete",
        other => anyhow::bail!("no self-row control is called `{other}`"),
    })
}

#[then(expr = "the {word} control on my own row is inert")]
async fn own_row_control_inert(world: &mut PingwardWorld, action: String) -> Result<()> {
    // The signed-in admin is always "admin" in this feature's Background.
    let row = user_row(world, "admin").await?;
    let control = row.test_id(self_row_test_id(&action)?).await?;
    let tag = tag_name(world, &control).await?;
    ensure!(tag == "SPAN", "the {action} control renders as <{tag}>");
    let classes = control.attr("class").await?.unwrap_or_default();
    ensure!(
        classes.split_whitespace().any(|class| class == "disabled"),
        "the {action} control's classes are {classes:?}"
    );
    Ok(())
}

#[then("the password reset control on my own row is usable")]
async fn own_row_reset_usable(world: &mut PingwardWorld) -> Result<()> {
    let row = user_row(world, "admin").await?;
    let control = row.test_id("user-reset-submit").await?;
    let tag = tag_name(world, &control).await?;
    ensure!(tag == "BUTTON", "the reset control renders as <{tag}>");
    ensure!(control.is_enabled().await?, "the reset control is disabled");
    Ok(())
}

#[then(expr = "the user {string} is listed with role {string}")]
async fn user_listed_with_role(
    world: &mut PingwardWorld,
    username: String,
    role: String,
) -> Result<()> {
    let row = user_row(world, &username).await?;
    let rendered = row.test_id("user-role").await?.normalized_text().await?;
    ensure!(rendered == role, "{username} is listed as {rendered:?}");
    Ok(())
}

#[then(expr = "the user {string} is marked disabled")]
async fn user_marked_disabled(world: &mut PingwardWorld, username: String) -> Result<()> {
    let row = user_row(world, &username).await?;
    let pill = row.test_id_opt("user-disabled").await?;
    ensure!(pill.is_some(), "{username} carries no disabled pill");
    Ok(())
}

#[then(expr = "the user {string} is not marked disabled")]
async fn user_not_marked_disabled(world: &mut PingwardWorld, username: String) -> Result<()> {
    // Asserting the row is there first keeps this from passing for a user who
    // was deleted rather than re-enabled.
    let row = user_row(world, &username).await?;
    let pill = row.test_id_opt("user-disabled").await?;
    ensure!(pill.is_none(), "{username} is still marked disabled");
    Ok(())
}

#[then(expr = "the user {string} is not listed")]
async fn user_not_listed(world: &mut PingwardWorld, username: String) -> Result<()> {
    world
        .driver()?
        .expect_absent(&format!("user-row-{username}"))
        .await
}

#[when(expr = "I try to add a user {string} with password {string}")]
async fn try_add_user(
    world: &mut PingwardWorld,
    username: String,
    password: String,
) -> Result<()> {
    // Unlike `add_user`, this expects the submission to be *refused*: the
    // handler re-renders `/admin` with an error instead of redirecting, so no
    // new row is expected.
    let driver = world.driver()?;
    driver.fill("user-username-input", &username).await?;
    driver.fill("user-password-input", &password).await?;
    driver.submit("user-submit").await
}

#[then(expr = "the user form shows the error {string}")]
async fn user_form_error(world: &mut PingwardWorld, message: String) -> Result<()> {
    world.driver()?.expect_exact_text("user-error", &message).await
}

#[when(expr = "I try to grant admin to {string}")]
async fn try_grant_admin(world: &mut PingwardWorld, username: String) -> Result<()> {
    // Like "I toggle admin on", but for the locked case, where the click does
    // *not* navigate: `app.js` intercepts it and opens the re-auth dialog. The
    // server-side bounce this stands in for (a browser without JS) is covered
    // in `tests/admin_elevation.rs`, where it is the real behaviour.
    let row = user_row(world, &username).await?;
    let control = row.test_id("user-toggle-admin").await?;
    click_when_ready(&control).await
}
