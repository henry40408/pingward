//! Notification channels: creating, editing, binding and delivering. The
//! channel form and rows mostly lack `data-testid`s, so these steps go by
//! input id, class and button text.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::dom::{Dom, TextContent, Within, click_when_ready, submit_element};
use pingward_e2e::wait::eventually_within;
use pingward_e2e::world::PingwardWorld;
use std::time::Duration;
use thirtyfour::WebElement;

/// How long to keep reloading while waiting for a delivery to be recorded.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(15);

/// Fills the channel form for `kind` with canned config and submits it. The
/// kind is picked first: `app.css` shows only that kind's inputs, and filling
/// needs a visible target.
async fn create_channel(
    world: &PingwardWorld,
    kind: &str,
    name: &str,
    url: Option<&str>,
) -> Result<()> {
    let project = world.project_url()?;
    world.goto(&format!("{project}/channels/new")).await?;
    let driver = world.driver()?;
    driver.fill_css("#name", name).await?;
    driver.select_option_css("#kind", kind).await?;
    match kind {
        "webhook" => {
            driver
                .fill_css("#webhook_url", url.unwrap_or("http://127.0.0.1:1/hook"))
                .await?;
        }
        "slack" => {
            driver
                .fill_css("#slack_url", "https://hooks.slack.com/services/T0/B0/xxx")
                .await?;
        }
        "telegram" => {
            driver.fill_css("#telegram_token", "123:ABC").await?;
            driver.fill_css("#telegram_chat_id", "999").await?;
        }
        "ntfy" => driver.fill_css("#ntfy_topic", "my-topic").await?,
        "pushover" => {
            driver.fill_css("#pushover_token", "apptok").await?;
            driver.fill_css("#pushover_user", "userkey").await?;
        }
        other => anyhow::bail!("unsupported channel kind in test: {other}"),
    }
    click_named_button(world, "Create channel").await?;
    world.expect_path_matching(r"/projects/\d+$").await
}

async fn click_named_button(world: &PingwardWorld, name: &str) -> Result<()> {
    let driver = world.driver()?;
    let button = driver
        .button_named(name)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the page has no `{name}` button"))?;
    submit_element(driver, &button).await
}

async fn bind_checkbox(world: &PingwardWorld, name: &str) -> Result<WebElement> {
    let row = world.driver()?.css_row("label.chk", name).await?;
    row.css_opt(r#"input[name="channel_ids"]"#)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the row for {name:?} has no bind checkbox"))
}

// cucumber-rs matches on the Gherkin keyword; this step is used as both.
#[given(expr = "I create a {word} channel named {string}")]
#[when(expr = "I create a {word} channel named {string}")]
async fn create_named_channel(world: &mut PingwardWorld, kind: String, name: String) -> Result<()> {
    create_channel(world, &kind, &name, None).await
}

#[given(expr = "a webhook channel named {string} targeting the mock server")]
async fn webhook_channel_to_mock(world: &mut PingwardWorld, name: String) -> Result<()> {
    // Recorded so the edit form can be checked for *not* printing it.
    let url = format!("{}/hook", world.mock_webhook().await?.url());
    world.webhook_url = Some(url.clone());
    create_channel(world, "webhook", &name, Some(&url)).await
}

#[then(expr = "the project lists a channel named {string} of kind {string}")]
async fn project_lists_channel(
    world: &mut PingwardWorld,
    name: String,
    kind: String,
) -> Result<()> {
    let row = world.driver()?.css_row(".chk", &name).await?;
    let rendered = row
        .css_opt(".kind")
        .await?
        .ok_or_else(|| anyhow::anyhow!("the row for {name:?} shows no kind"))?
        .normalized_text()
        .await?;
    ensure!(rendered == kind, "{name} is listed as kind {rendered:?}");
    Ok(())
}

#[when("I submit a webhook channel with a blank URL")]
async fn submit_blank_webhook(world: &mut PingwardWorld) -> Result<()> {
    let project = world.project_url()?;
    world.goto(&format!("{project}/channels/new")).await?;
    let driver = world.driver()?;
    driver.fill_css("#name", "bad hook").await?;
    driver.select_option_css("#kind", "webhook").await?;
    click_named_button(world, "Create channel").await
}

#[then(expr = "the channel form shows an error {string}")]
async fn channel_form_error(world: &mut PingwardWorld, message: String) -> Result<()> {
    world
        .driver()?
        .expect_exact_text_css(".flash.err", &message)
        .await
}

#[when("I open the new channel form")]
async fn open_new_channel_form(world: &mut PingwardWorld) -> Result<()> {
    let project = world.project_url()?;
    world.goto(&format!("{project}/channels/new")).await
}

#[then(expr = "the {string} channel kind is not offered")]
async fn kind_not_offered(world: &mut PingwardWorld, kind: String) -> Result<()> {
    world
        .driver()?
        .expect_count(&format!("#kind option[value=\"{kind}\"]"), 0)
        .await
}

#[when(expr = "I delete the channel named {string}")]
async fn delete_channel(world: &mut PingwardWorld, name: String) -> Result<()> {
    // Redirects back to the same URL, so `submit_element` waits on the
    // document being replaced rather than on the URL.
    let driver = world.driver()?;
    let row = driver.css_row(".chk", &name).await?;
    let button = row
        .button_named("delete")
        .await?
        .ok_or_else(|| anyhow::anyhow!("the row for {name:?} has no delete button"))?;
    submit_element(driver, &button).await
}

#[then("the project shows no channels")]
async fn project_shows_no_channels(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_visible("project-channels-empty").await?;
    driver
        .expect_text("project-channels-empty", "nobody is notified")
        .await
}

#[when(expr = "I open the edit form for the channel {string}")]
async fn open_channel_edit_form(world: &mut PingwardWorld, name: String) -> Result<()> {
    let driver = world.driver()?;
    let row = driver.css_row(".chk", &name).await?;
    let link = row
        .link_named("edit")
        .await?
        .ok_or_else(|| anyhow::anyhow!("the row for {name:?} has no edit link"))?;
    submit_element(driver, &link).await?;
    world.expect_path_matching(r"/channels/\d+/edit$").await
}

#[then("the edit form hides the stored webhook URL")]
async fn edit_form_hides_url(world: &mut PingwardWorld) -> Result<()> {
    // The kind assertion is the non-vacuity guard: a page rendering no
    // channel would pass every "absent" check.
    let driver = world.driver()?;
    driver
        .expect_exact_text("channel-kind-static", "webhook")
        .await?;
    driver.expect_value_css("#webhook_url", "").await?;
    driver
        .expect_attr("#webhook_url", "placeholder", Some("unchanged"))
        .await?;
    driver
        .expect_exact_text_css(".pill.ok", "configured")
        .await?;
    let stored = world.webhook_url()?;
    let body = driver
        .find(thirtyfour::By::Tag("body"))
        .await?
        .content_text()
        .await?;
    ensure!(
        !body.contains(&stored),
        "the edit form prints the stored webhook URL"
    );
    Ok(())
}

#[when(expr = "I rename the channel to {string}")]
async fn rename_channel(world: &mut PingwardWorld, name: String) -> Result<()> {
    world.driver()?.fill_css("#name", &name).await?;
    click_named_button(world, "Save changes").await?;
    world.expect_path_matching(r"/projects/\d+$").await
}

#[when("I change the channel's webhook URL to the mock server")]
async fn change_channel_url_to_mock(world: &mut PingwardWorld) -> Result<()> {
    let url = format!("{}/hook", world.mock_webhook().await?.url());
    world.webhook_url = Some(url.clone());
    world.driver()?.fill_css("#webhook_url", &url).await?;
    click_named_button(world, "Save changes").await?;
    world.expect_path_matching(r"/projects/\d+$").await
}

#[then(expr = "the kind is shown as static text {string}")]
async fn kind_is_static(world: &mut PingwardWorld, kind: String) -> Result<()> {
    let driver = world.driver()?;
    driver
        .expect_exact_text("channel-kind-static", &kind)
        .await?;
    driver.expect_count("#kind", 0).await
}

#[given(expr = "I bind the channel {string} to the check")]
#[when(expr = "I bind the channel {string} to the check")]
async fn bind_channel(world: &mut PingwardWorld, name: String) -> Result<()> {
    // Saving redirects to the same URL; `submit_element` waits on the document.
    let checkbox = bind_checkbox(world, &name).await?;
    if !checkbox.is_selected().await? {
        click_when_ready(&checkbox).await?;
    }
    click_named_button(world, "Save channels").await
}

#[then(expr = "the channel {string} is bound to the check")]
async fn channel_is_bound(world: &mut PingwardWorld, name: String) -> Result<()> {
    let checkbox = bind_checkbox(world, &name).await?;
    ensure!(
        checkbox.is_selected().await?,
        "the channel {name:?} is not ticked"
    );
    Ok(())
}

#[then(expr = "a {string} confirmation is shown")]
async fn confirmation_shown(world: &mut PingwardWorld, message: String) -> Result<()> {
    world
        .driver()?
        .expect_exact_text("check-flash", &message)
        .await
}

#[then("the confirmation is gone after reloading")]
async fn confirmation_gone_after_reload(world: &mut PingwardWorld) -> Result<()> {
    // One-shot: the flash cookie is cleared on the render that shows it.
    world.driver()?.refresh().await?;
    world.driver()?.expect_absent("check-flash").await
}

#[when(expr = "I send a test notification to the channel {string}")]
async fn send_test_notification(world: &mut PingwardWorld, name: String) -> Result<()> {
    // Renders the project page directly (no redirect); the next assertion is
    // what waits for the banner.
    let driver = world.driver()?;
    let row = driver.css_row(".chk", &name).await?;
    let button = row
        .button_named("Send test")
        .await?
        .ok_or_else(|| anyhow::anyhow!("the row for {name:?} has no `Send test` button"))?;
    click_when_ready(&button).await
}

#[then("a channel success banner is shown")]
async fn channel_success_banner(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible_css(".flash.ok").await
}

#[then("a channel error banner is shown")]
async fn channel_error_banner(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible_css(".flash.err").await
}

#[then(expr = "the mock server receives a {string} notification")]
async fn mock_receives(world: &mut PingwardWorld, event: String) -> Result<()> {
    world.mock_webhook().await?.wait_for_payload(&event).await?;
    Ok(())
}

#[then(
    expr = "the {string} notification payload names project {string}, links the check, and blames {string}"
)]
async fn payload_is_enriched(
    world: &mut PingwardWorld,
    event: String,
    project: String,
    cause: String,
) -> Result<()> {
    let base_url = world.base_url()?.to_owned();
    let payload = world.mock_webhook().await?.wait_for_payload(&event).await?;
    let field = |name: &str| {
        payload
            .get(name)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    ensure!(
        field("project") == project,
        "the payload names project {:?}",
        field("project")
    );
    ensure!(
        field("cause") == cause,
        "the payload blames {:?}",
        field("cause")
    );
    let url = field("url");
    let links_check =
        regex::Regex::new(&format!(r"^{}/checks/\d+$", regex::escape(&base_url)))?.is_match(&url);
    ensure!(links_check, "the payload links {url:?}");
    ensure!(
        field("schedule") == "every 1h (grace 5m)",
        "the payload's schedule reads {:?}",
        field("schedule")
    );
    ensure!(
        field("text").contains(&url),
        "the message body does not carry the link"
    );
    Ok(())
}

#[then("the check's notify channels show an empty state")]
async fn check_channels_empty(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible("check-channels-empty").await
}

#[when(expr = "I visit the check page for {string}")]
async fn visit_check_page_for(world: &mut PingwardWorld, name: String) -> Result<()> {
    // Clicks the row, not the name link, to exercise `app.js`'s `data-href`.
    let driver = world.driver()?;
    let row = driver.css_row(".check", &name).await?;
    submit_element(driver, &row).await?;
    world.expect_path_matching(r"/checks/\d+$").await
}

#[when("I visit the dashboard")]
async fn visit_dashboard(world: &mut PingwardWorld) -> Result<()> {
    world.goto("/").await
}

#[then(expr = "the channel {string} shows as ON on the check page")]
async fn channel_shows_on(world: &mut PingwardWorld, name: String) -> Result<()> {
    channel_state(world, &name, true).await
}

#[then(expr = "the channel {string} shows as OFF on the check page")]
async fn channel_shows_off(world: &mut PingwardWorld, name: String) -> Result<()> {
    channel_state(world, &name, false).await
}

/// `channel-state-N` wraps both `.on` and `.off` and CSS shows one, so
/// visibility is asserted: `textContent` reads "ONOFF" either way.
async fn channel_state(world: &PingwardWorld, name: &str, on: bool) -> Result<()> {
    let row = world.driver()?.css_row("label.chk", name).await?;
    let state = row
        .css_opt(r#"[data-testid^="channel-state-"]"#)
        .await?
        .ok_or_else(|| anyhow::anyhow!("the row for {name:?} has no state marker"))?;
    let shown = state
        .css_opt(if on { ".on" } else { ".off" })
        .await?
        .ok_or_else(|| anyhow::anyhow!("the state marker has no span"))?;
    let hidden = state
        .css_opt(if on { ".off" } else { ".on" })
        .await?
        .ok_or_else(|| anyhow::anyhow!("the state marker has no counterpart span"))?;
    ensure!(
        shown.is_displayed().await?,
        "the channel does not read {}",
        if on { "ON" } else { "OFF" }
    );
    ensure!(
        !hidden.is_displayed().await?,
        "both states are displayed at once"
    );
    Ok(())
}

#[then(expr = "the dashboard shows a {string} chip for the check {string}")]
async fn dashboard_shows_chip(world: &mut PingwardWorld, chip: String, name: String) -> Result<()> {
    let row = dashboard_row(world, &name).await?;
    let rendered = row
        .test_id("check-no-channel")
        .await?
        .normalized_text()
        .await?;
    ensure!(
        rendered == chip,
        "the check {name:?} has no bound channel, so its row must carry the {chip:?} chip; \
         it reads {rendered:?}"
    );
    Ok(())
}

#[then(expr = "the dashboard shows no {string} chip for the check {string}")]
async fn dashboard_hides_chip(world: &mut PingwardWorld, chip: String, name: String) -> Result<()> {
    // `dashboard_row` erroring on a missing row is the non-vacuity guard.
    let row = dashboard_row(world, &name).await?;
    ensure!(
        row.test_id_opt("check-no-channel").await?.is_none(),
        "the check {name:?} is bound to a channel, so its row must not carry the chip"
    );
    // The wording too, in case the warning moves to another test id.
    let text = row.normalized_text().await?;
    ensure!(
        !text.contains(&chip),
        "the check {name:?} is bound to a channel, yet its row still reads {chip:?}"
    );
    Ok(())
}

async fn dashboard_row(world: &PingwardWorld, name: &str) -> Result<WebElement> {
    world
        .driver()?
        .test_ids_with_text("dashboard-check-row", name)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no dashboard row for the check {name:?}"))
}

#[then(expr = "the check's recent notifications show a delivery to {string}")]
async fn notifications_show_delivery(world: &mut PingwardWorld, channel: String) -> Result<()> {
    // The row is recorded after the webhook POST returns, so reload until it
    // appears.
    let driver = world.driver()?;
    eventually_within(
        DELIVERY_TIMEOUT,
        &format!("a recorded delivery to {channel}"),
        || async {
            driver.refresh().await?;
            for row in driver.css_with_text("tr", &channel).await? {
                if row.normalized_text().await?.contains("sent") {
                    return Ok(true);
                }
            }
            Ok(false)
        },
    )
    .await
}

/// The `.pill.{class}` the recent-notifications table renders for `event`.
fn event_pill_class(event: &str) -> Result<&'static str> {
    Ok(match event {
        "down" => "fail",
        "up" => "ok",
        "reminder" => "start",
        other => anyhow::bail!("no notification pill is called `{other}`"),
    })
}

#[then(expr = "the recent notifications table shows a {string} event")]
async fn notifications_show_event(world: &mut PingwardWorld, event: String) -> Result<()> {
    // Scoped so a ping's `.pill.fail` cannot satisfy a "down" event.
    world
        .driver()?
        .expect_visible_css(&format!(
            "#notifs-section .pill.{}",
            event_pill_class(&event)?
        ))
        .await
}
