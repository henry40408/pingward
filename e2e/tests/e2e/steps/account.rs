//! The per-user account page: sessions, password and API keys.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::dom::{Dom, TextContent};
use pingward_e2e::world::PingwardWorld;

#[when("I open the account page")]
async fn open_account_page(world: &mut PingwardWorld) -> Result<()> {
    world.goto("/account").await
}

#[then("the current session is marked as this device")]
async fn current_session_marked(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible("session-current").await
}

#[when("I revoke the current session")]
async fn revoke_current_session(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.confirm_and_submit("session-revoke").await
}

#[when(expr = "I change my password from {string} to {string}")]
async fn change_password(world: &mut PingwardWorld, current: String, next: String) -> Result<()> {
    let driver = world.driver()?;
    driver.fill("current-password-input", &current).await?;
    driver.fill("new-password-input", &next).await?;
    driver.fill("confirm-password-input", &next).await?;
    driver.submit("password-submit").await
}

#[then("the password change is confirmed")]
async fn password_change_confirmed(world: &mut PingwardWorld) -> Result<()> {
    world
        .driver()?
        .expect_visible("password-changed-flash")
        .await
}

#[then("the password change is rejected")]
async fn password_change_rejected(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_visible("password-error").await?;
    driver.expect_absent("password-changed-flash").await
}

#[when(expr = "I create an API key named {string} with my password {string}")]
async fn create_api_key(world: &mut PingwardWorld, name: String, password: String) -> Result<()> {
    // Minting a key re-authenticates, since the key outlives the session that
    // created it.
    let driver = world.driver()?;
    driver.fill("api-key-name-input", &name).await?;
    driver.fill("api-key-password-input", &password).await?;
    driver.submit("api-key-submit").await
}

#[then("the API key creation is rejected")]
async fn api_key_rejected(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_visible("api-key-error").await?;
    driver.expect_absent("api-key-token").await
}

#[then("the new API key token is shown once")]
async fn api_key_token_shown(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_visible("api-key-token").await?;
    let token = driver.text_of("api-key-token").await?;
    let well_formed = regex::Regex::new("^pw_[0-9a-f]{64}$")
        .expect("the token pattern compiles")
        .is_match(&token);
    ensure!(well_formed, "the token reads {token:?}");
    Ok(())
}

#[then(expr = "the API keys list shows a key named {string}")]
async fn api_keys_list_shows(world: &mut PingwardWorld, name: String) -> Result<()> {
    world.driver()?.expect_exact_text_somewhere(&name).await
}

#[when("I revoke the API key")]
async fn revoke_api_key(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.confirm_and_submit("api-key-delete").await
}

#[then("no API keys remain")]
async fn no_api_keys(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible("api-keys-empty").await
}

#[given(expr = "requests arrive through a trusted proxy as {string}")]
async fn requests_through_proxy(world: &mut PingwardWorld, ip: String) -> Result<()> {
    // Every later request carries the header a proxy would add. Only honoured
    // alongside the `@trusted-proxy` tag.
    world
        .browser()?
        .set_extra_headers(serde_json::json!({ "x-forwarded-for": ip }))
        .await
}

#[then(expr = "the current session shows the IP {string}")]
async fn current_session_shows_ip(world: &mut PingwardWorld, ip: String) -> Result<()> {
    // Covers what `auth::client_ip`'s unit tests cannot: that the login handler
    // calls it and stores the result, rather than the raw socket peer.
    let driver = world.driver()?;
    driver.expect_visible("session-current").await?;
    let row = driver
        .find(thirtyfour::By::XPath(
            "//tr[.//*[@data-testid='session-current']]",
        ))
        .await?;
    let text = row.normalized_text().await?;
    ensure!(text.contains(&ip), "the current session row reads {text:?}");
    Ok(())
}
