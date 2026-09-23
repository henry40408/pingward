//! UI actions shared across step modules.

use anyhow::Result;

use crate::dom::Dom;
use crate::world::PingwardWorld;

/// Submits the `/login` form; callers assert where it lands. Signs out first
/// if needed, since `/login` bounces an authenticated visitor to `/`.
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

/// Reveals the ping URL if withheld (an admin on someone else's check must
/// reveal it, see `CheckPageViewer` in `src/web.rs`).
pub async fn reveal_ping_url_if_withheld(world: &PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    if driver.test_id_opt("reveal-ping-url").await?.is_some() {
        driver.submit("reveal-ping-url").await?;
        driver.expect_visible("ping-url").await?;
    }
    Ok(())
}

/// Reads the check page's ping URL, revealing it first when withheld.
pub async fn read_ping_url(world: &PingwardWorld) -> Result<String> {
    reveal_ping_url_if_withheld(world).await?;
    world.driver()?.text_of("ping-url").await
}
