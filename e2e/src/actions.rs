//! UI actions shared across step modules.

use anyhow::Result;

use crate::dom::Dom;
use crate::world::PingwardWorld;

/// Drives the `/login` form, stopping at the click: callers assert the
/// destination themselves, since a failed login stays on `/login`.
///
/// Switching accounts needs the explicit sign-out — `/login` bounces an
/// already-authenticated visitor to `/`, so the form is only reachable while
/// signed out.
///
/// # Errors
///
/// Fails when the page cannot be reached or the form cannot be driven.
pub async fn sign_in(world: &PingwardWorld, username: &str, password: &str) -> Result<()> {
    world.goto("/login").await?;
    if !world.path().await?.ends_with("/login") {
        world.driver()?.submit("logout-button").await?;
        world.goto("/login").await?;
    }
    let driver = world.driver()?;
    driver.fill("username-input", username).await?;
    driver.fill("password-input", password).await?;
    driver.submit("login-submit").await?;
    Ok(())
}

/// Reveals the ping URL when the page is withholding it.
///
/// An admin viewing someone else's check gets it only after an audited reveal
/// (`CheckPageViewer` in `src/web.rs`). Steps that merely need the URL go
/// through this, so they read the same on the owner and admin routes.
///
/// # Errors
///
/// Fails when the reveal is offered but does not produce a URL.
pub async fn reveal_ping_url_if_withheld(world: &PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    if driver.test_id_opt("reveal-ping-url").await?.is_some() {
        driver.submit("reveal-ping-url").await?;
        driver.expect_visible("ping-url").await?;
    }
    Ok(())
}

/// Reads the check page's ping URL, revealing it first when withheld.
///
/// # Errors
///
/// Fails when the page shows no ping URL.
pub async fn read_ping_url(world: &PingwardWorld) -> Result<String> {
    reveal_ping_url_if_withheld(world).await?;
    world.driver()?.text_of("ping-url").await
}
