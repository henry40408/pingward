//! Setup, sign-in and sign-out — a port of `auth.steps.js`.

use anyhow::Result;
use cucumber::{given, then, when};
use pingward_e2e::actions::sign_in;
use pingward_e2e::dom::Dom;
use pingward_e2e::world::PingwardWorld;

#[given(expr = "I visit {string}")]
#[when(expr = "I visit {string}")]
async fn visit(world: &mut PingwardWorld, path: String) -> Result<()> {
    world.goto(&path).await
}

#[then("I am on the setup page")]
async fn on_setup_page(world: &mut PingwardWorld) -> Result<()> {
    world.expect_path("/setup").await?;
    world.driver()?.expect_visible("setup-submit").await
}

#[when(expr = "I create the admin {string} with password {string}")]
async fn create_admin(world: &mut PingwardWorld, username: String, password: String) -> Result<()> {
    let driver = world.driver()?;
    driver.fill("username-input", &username).await?;
    driver.fill("password-input", &password).await?;
    driver.submit("setup-submit").await
}

#[then("I land on the dashboard signed in")]
async fn land_on_dashboard(world: &mut PingwardWorld) -> Result<()> {
    world.expect_path("/").await?;
    world.driver()?.expect_visible("logout-button").await
}

#[given(expr = "an admin {string} with password {string} exists")]
async fn admin_exists(world: &mut PingwardWorld, username: String, password: String) -> Result<()> {
    world.api()?.bootstrap_admin(&username, &password).await
}

#[when(expr = "I sign in as {string} with password {string}")]
async fn sign_in_as(world: &mut PingwardWorld, username: String, password: String) -> Result<()> {
    sign_in(world, &username, &password).await
}

#[given(expr = "I am signed in as {string} with password {string}")]
async fn signed_in_as(world: &mut PingwardWorld, username: String, password: String) -> Result<()> {
    sign_in(world, &username, &password).await?;
    world.expect_path("/").await
}

#[then(expr = "the login page shows the error {string}")]
async fn login_error(world: &mut PingwardWorld, message: String) -> Result<()> {
    world.expect_path("/login").await?;
    world.driver()?.expect_text("login-error", &message).await
}

#[when("I sign out")]
async fn sign_out(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.submit("logout-button").await
}

#[then("I am on the login page")]
async fn on_login_page(world: &mut PingwardWorld) -> Result<()> {
    world.expect_path("/login").await?;
    world.driver()?.expect_visible("login-submit").await
}
