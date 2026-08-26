//! Authorization boundaries: the admin nav, cross-user reads and the CSRF
//! guard.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::actions::sign_in;
use pingward_e2e::dom::Dom;
use pingward_e2e::world::PingwardWorld;

/// The human nav label the features use, and the `data-testid` behind it.
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
    // Created through the admin-only "Add user" form, which assumes the admin
    // is signed in; the unticked is_admin checkbox makes the account a member.
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

// Reads what an earlier step recorded, so there is nothing to await.
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
    // Issued from inside the page, so the session cookie rides along and the
    // request reaches the CSRF guard as a logged-in one missing only the token.
    // The scenario asserts a live session first, so the 403 is attributable.
    world.status = Some(world.fetch_status("POST", &path).await?);
    Ok(())
}

#[given("I remember the current project")]
async fn remember_current_project(world: &mut PingwardWorld) -> Result<()> {
    // The project-creating step submits without awaiting the redirect, so
    // reading the path immediately could still see `/projects/new`.
    world.expect_path_matching(r"/projects/\d+$").await?;
    world.project_url = Some(world.path().await?);
    Ok(())
}

#[given("the owner can read the remembered project")]
async fn owner_can_read_project(world: &mut PingwardWorld) -> Result<()> {
    // The positive control for the cross-user 404: the owner gets 200, so the
    // later 404 is attributable to the ownership guard rather than a broken
    // route.
    let project = world.project_url()?;
    world.goto(&project).await?;
    let status = world.fetch_status("GET", &project).await?;
    ensure!(status == 200, "the owner's own project answered {status}");
    Ok(())
}

#[when(expr = "I revisit it as {string} with password {string}")]
async fn revisit_as(world: &mut PingwardWorld, username: String, password: String) -> Result<()> {
    // Expected 404: the project exists, but `owned_project` hides it from
    // anyone but its owner.
    world.driver()?.submit("logout-button").await?;
    sign_in(world, &username, &password).await?;
    world.expect_path("/").await?;
    let project = world.project_url()?;
    world.goto(&project).await?;
    world.status = Some(world.fetch_status("GET", &project).await?);
    Ok(())
}
