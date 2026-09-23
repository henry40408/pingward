//! Authorization boundaries: the admin nav, cross-user reads and the CSRF
//! guard.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::actions::sign_in;
use pingward_e2e::dom::Dom;
use pingward_e2e::world::PingwardWorld;

fn nav_test_id(label: &str) -> Result<&'static str> {
    Ok(match label {
        "Admin" => "nav-admin",
        other => anyhow::bail!("no nav link is labelled `{other}`"),
    })
}

#[given(expr = "a non-admin user {string} with password {string} exists")]
async fn non_admin_user_exists(
    world: &mut PingwardWorld,
    username: String,
    password: String,
) -> Result<()> {
    // Via the admin's "Add user" form, so the admin must be signed in.
    world.goto("/admin").await?;
    let driver = world.driver()?;
    driver.fill("user-username-input", &username).await?;
    driver.fill("user-password-input", &password).await?;
    driver.submit("user-submit").await?;
    world.expect_path("/admin").await?;
    world.driver()?.expect_exact_text_somewhere(&username).await
}

#[given(expr = "the {string} nav link is visible")]
#[then(expr = "the {string} nav link is visible")]
async fn nav_link_visible(world: &mut PingwardWorld, label: String) -> Result<()> {
    world.driver()?.expect_visible(nav_test_id(&label)?).await
}

#[then(expr = "the {string} nav link is not visible")]
async fn nav_link_absent(world: &mut PingwardWorld, label: String) -> Result<()> {
    world.driver()?.expect_absent(nav_test_id(&label)?).await
}

#[when(expr = "I navigate to {string}")]
async fn navigate_to(world: &mut PingwardWorld, path: String) -> Result<()> {
    world.goto(&path).await?;
    world.status = Some(world.fetch_status("GET", &path).await?);
    Ok(())
}

#[then(expr = "the response status is {int}")]
fn response_status(world: &mut PingwardWorld, status: u16) -> Result<()> {
    let seen = world
        .status
        .ok_or_else(|| anyhow::anyhow!("no step recorded a response status"))?;
    ensure!(seen == status, "the response was {seen}, not {status}");
    Ok(())
}

#[when(expr = "I POST to {string} without a CSRF token")]
async fn post_without_csrf(world: &mut PingwardWorld, path: String) -> Result<()> {
    // A page `fetch` with the session cookie, missing only the token; the
    // scenario asserts a live session first so the 403 is attributable.
    world.status = Some(world.fetch_status("POST", &path).await?);
    Ok(())
}

#[given("I remember the current project")]
async fn remember_current_project(world: &mut PingwardWorld) -> Result<()> {
    // "I create a project named" does not await the redirect.
    world.expect_path_matching(r"/projects/\d+$").await?;
    world.project_url = Some(world.path().await?);
    Ok(())
}

#[given("the owner can read the remembered project")]
async fn owner_can_read_project(world: &mut PingwardWorld) -> Result<()> {
    // Positive control, so the later 404 is the ownership guard, not the route.
    let project = world.project_url()?;
    world.goto(&project).await?;
    let status = world.fetch_status("GET", &project).await?;
    ensure!(status == 200, "the owner's own project answered {status}");
    Ok(())
}

#[when(expr = "I revisit it as {string} with password {string}")]
async fn revisit_as(world: &mut PingwardWorld, username: String, password: String) -> Result<()> {
    // `owned_project` answers 404 to a non-owner.
    world.driver()?.submit("logout-button").await?;
    sign_in(world, &username, &password).await?;
    world.expect_path("/").await?;
    let project = world.project_url()?;
    world.goto(&project).await?;
    world.status = Some(world.fetch_status("GET", &project).await?);
    Ok(())
}
