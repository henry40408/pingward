//! The merged `/admin` page: cross-user access, the environment card, the
//! audit trail and the elevation gate.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::actions::sign_in;
use pingward_e2e::dom::{Dom, TextContent, click_when_ready, submit_element};
use pingward_e2e::world::PingwardWorld;

/// Captures a trailing row id out of a path.
fn id_from(path: &str, pattern: &str) -> Result<i64> {
    let captures = regex::Regex::new(pattern)?
        .captures(path)
        .ok_or_else(|| anyhow::anyhow!("`{path}` does not match {pattern}"))?;
    Ok(captures[1].parse()?)
}

#[given(
    expr = "{string} with password {string} owns a project {string} with a check {string} period {int}"
)]
async fn member_owns_project_and_check(
    world: &mut PingwardWorld,
    username: String,
    password: String,
    project: String,
    check: String,
    period: i64,
) -> Result<()> {
    // Seeds a project and check owned by a second, non-admin user, so the
    // `/admin/*` scenarios exercise real cross-user access. We arrive signed in
    // as the admin; `sign_in` handles the switch, since `/login` bounces an
    // authenticated visitor to `/`.
    sign_in(world, &username, &password).await?;
    world.expect_path("/").await?;

    world.goto("/projects/new").await?;
    let driver = world.driver()?;
    driver.fill("project-name-input", &project).await?;
    driver.submit("project-submit").await?;
    world.expect_path_matching(r"/projects/\d+$").await?;
    world.project_id = Some(id_from(&world.path().await?, r"/projects/(\d+)$")?);

    let driver = world.driver()?;
    driver.submit("new-check-link").await?;
    driver.fill("check-name-input", &check).await?;
    driver
        .fill("check-period-input", &period.to_string())
        .await?;
    driver.submit("check-submit").await?;
    world.expect_path_matching(r"/checks/\d+$").await?;
    world.check_id = Some(id_from(&world.path().await?, r"/checks/(\d+)$")?);
    Ok(())
}

#[when("I open the admin dashboard")]
#[when("I open the admin projects list")]
async fn open_admin(world: &mut PingwardWorld) -> Result<()> {
    // "All projects" is a section of the merged `/admin`, not its own page.
    world.goto("/admin").await
}

#[then("the admin dashboard is shown")]
async fn admin_dashboard_shown(world: &mut PingwardWorld) -> Result<()> {
    // Section headings carry no `data-testid`, so they are matched by text;
    // the site-wide counts render as bare tiles and are matched by test id.
    let driver = world.driver()?;
    let heading = driver.heading_opt("Admin").await?;
    ensure!(heading.is_some(), "no `Admin` heading is rendered");
    driver
        .expect_count("[data-testid=\"admin-scale\"] .tile", 4)
        .await
}

#[then("no card subheading renders larger than its card heading")]
async fn subheads_are_subordinate(world: &mut PingwardWorld) -> Result<()> {
    // A `.subhead` must read below the card's own `.ch h2`. Compares computed
    // sizes, not declarations — the bug it guards was an inherited global `h2`.
    // The 2px allowance lets a subhead exceed the uppercase, letter-spaced card
    // label slightly without reopening the 21px-vs-13px gap.
    let measured = world
        .driver()?
        .eval(
            "const size = (el) => parseFloat(getComputedStyle(el).fontSize);\
             const heading = size(document.querySelector('.card .ch h2'));\
             const subheads = [...document.querySelectorAll('.card .subhead')].map((el) => ({\
               text: el.textContent.trim(), size: size(el),\
             }));\
             return { heading, subheads };",
        )
        .await?;
    let heading = measured
        .get("heading")
        .and_then(serde_json::Value::as_f64)
        .ok_or_else(|| anyhow::anyhow!("no card heading was measured"))?;
    let subheads = measured
        .get("subheads")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    ensure!(
        !subheads.is_empty(),
        "no .subhead rendered — the check would be vacuous"
    );
    for subhead in subheads {
        let size = subhead
            .get("size")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or_default();
        let text = subhead
            .get("text")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        ensure!(
            size <= heading + 2.0,
            "subheading {text:?} renders at {size}px inside a {heading}px card heading"
        );
    }
    Ok(())
}

#[then(expr = "the admin projects list shows {string} owned by {string}")]
async fn admin_projects_list_shows(
    world: &mut PingwardWorld,
    project: String,
    owner: String,
) -> Result<()> {
    let driver = world.driver()?;
    let heading = driver.heading_opt("All projects").await?;
    ensure!(heading.is_some(), "no `All projects` heading is rendered");
    let row = driver.css_row(".check", &project).await?;
    let text = row.normalized_text().await?;
    ensure!(
        text.contains(&format!("owner: {owner}")),
        "the row for {project:?} reads {text:?}"
    );
    Ok(())
}

#[given("I open the member's project in the admin area")]
async fn open_member_project(world: &mut PingwardWorld) -> Result<()> {
    // These are the shared owner templates with `/admin`-prefixed forms, so
    // the steps that follow reuse the monitoring definitions verbatim.
    let id = world
        .project_id
        .ok_or_else(|| anyhow::anyhow!("no step recorded the member's project"))?;
    world.goto(&format!("/admin/projects/{id}")).await
}

#[given("I open the member's check in the admin area")]
#[when("I open the member's check in the admin area")]
async fn open_member_check(world: &mut PingwardWorld) -> Result<()> {
    let id = world
        .check_id
        .ok_or_else(|| anyhow::anyhow!("no step recorded the member's check"))?;
    world.goto(&format!("/admin/checks/{id}")).await
}

#[then(expr = "I am viewing the check {string}")]
async fn viewing_check(world: &mut PingwardWorld, name: String) -> Result<()> {
    // Both `project.html` and `check.html` render the name as the page `<h1>`.
    let heading = world.driver()?.heading_opt(&name).await?;
    ensure!(heading.is_some(), "no heading reads {name:?}");
    Ok(())
}

#[when(expr = "I rename the project to {string}")]
async fn rename_project(world: &mut PingwardWorld, name: String) -> Result<()> {
    let id = world
        .project_id
        .ok_or_else(|| anyhow::anyhow!("no step recorded the member's project"))?;
    world.goto(&format!("/admin/projects/{id}/edit")).await?;
    let driver = world.driver()?;
    driver.fill("project-name-input", &name).await?;
    driver.submit("project-submit").await
}

#[then(expr = "I am on the admin project page for {string}")]
async fn on_admin_project_page(world: &mut PingwardWorld, name: String) -> Result<()> {
    let id = world
        .project_id
        .ok_or_else(|| anyhow::anyhow!("no step recorded the member's project"))?;
    world.expect_path(&format!("/admin/projects/{id}")).await?;
    let heading = world.driver()?.heading_opt(&name).await?;
    ensure!(heading.is_some(), "no heading reads {name:?}");
    Ok(())
}

#[when(expr = "I add a webhook channel named {string}")]
async fn add_webhook_channel(world: &mut PingwardWorld, name: String) -> Result<()> {
    // `channel_form.html` carries no `data-testid`, so fields are addressed by
    // id. Webhook is the default kind, so only name and URL need filling.
    let id = world
        .project_id
        .ok_or_else(|| anyhow::anyhow!("no step recorded the member's project"))?;
    world
        .goto(&format!("/admin/projects/{id}/channels/new"))
        .await?;
    let driver = world.driver()?;
    driver.fill_css("#name", &name).await?;
    driver
        .fill_css("#webhook_url", "https://example.com/hook")
        .await?;
    let create = driver
        .button_named("Create channel")
        .await?
        .ok_or_else(|| anyhow::anyhow!("the channel form has no `Create channel` button"))?;
    submit_element(driver, &create).await?;
    world.expect_path(&format!("/admin/projects/{id}")).await
}

#[then(expr = "the channel {string} is listed on the project")]
async fn channel_listed(world: &mut PingwardWorld, name: String) -> Result<()> {
    world.driver()?.css_row(".chk .nm", &name).await.map(|_| ())
}

#[when("I delete the member's project")]
async fn delete_member_project(world: &mut PingwardWorld) -> Result<()> {
    // The admin delete redirects to `/admin` where the owner flow redirects to
    // the dashboard, hence its own step.
    world
        .driver()?
        .confirm_and_submit("delete-project-button")
        .await?;
    world.expect_path("/admin").await
}

#[then("the admin projects list has no projects")]
async fn admin_projects_empty(world: &mut PingwardWorld) -> Result<()> {
    world
        .driver()?
        .expect_text_somewhere("No projects yet.")
        .await
}

#[then("the Environment card shows the SMTP password as configured")]
async fn env_smtp_password_configured(world: &mut PingwardWorld) -> Result<()> {
    world
        .driver()?
        .expect_text("env-smtp-password", "configured")
        .await
}

#[then("the page does not contain the SMTP secret")]
async fn page_hides_smtp_secret(world: &mut PingwardWorld) -> Result<()> {
    let body = world
        .driver()?
        .find(thirtyfour::By::Tag("body"))
        .await?
        .content_text()
        .await?;
    ensure!(
        !body.contains("e2e-secret-password"),
        "the page prints the configured SMTP password"
    );
    Ok(())
}

#[then(expr = "the audit trail has at least {int} row")]
async fn audit_has_rows(world: &mut PingwardWorld, count: usize) -> Result<()> {
    let driver = world.driver()?;
    pingward_e2e::wait::eventually(&format!("the audit trail has {count} row(s)"), || async {
        Ok(driver.test_ids("audit-row").await?.len() >= count)
    })
    .await
}

#[then(expr = "the audit trail shows an {string} entry")]
async fn audit_shows_entry(world: &mut PingwardWorld, action: String) -> Result<()> {
    let driver = world.driver()?;
    pingward_e2e::wait::eventually(&format!("an `{action}` audit entry"), || async {
        Ok(!driver
            .test_ids_with_text("audit-row", &action)
            .await?
            .is_empty())
    })
    .await
}

#[when("I expand the first audit row")]
async fn expand_first_audit_row(world: &mut PingwardWorld) -> Result<()> {
    // A row with a request behind it is a `tr.toggle` followed by a `tr.exp`
    // that gains `.open` on click, as the ping table's captured output does.
    world.driver()?.click("audit-row").await
}

#[then("the audit detail shows the request path")]
async fn audit_detail_shows_path(world: &mut PingwardWorld) -> Result<()> {
    world
        .driver()?
        .expect_text_css("#audit-section tr.exp.open", "/admin/")
        .await
}

#[when(expr = "I filter the audit trail by action {string}")]
async fn filter_audit_by_action(world: &mut PingwardWorld, action: String) -> Result<()> {
    let driver = world.driver()?;
    driver.select_option("audit-action", &action).await?;
    driver.click("audit-apply").await?;
    // The Clear link renders only in a filtered response, so waiting for it
    // stops the next assertions racing the pre-filter rows.
    driver.expect_visible("audit-clear").await
}

#[when(expr = "I filter the audit trail by actor {string}")]
async fn filter_audit_by_actor(world: &mut PingwardWorld, actor: String) -> Result<()> {
    // The select is built with `SELECT DISTINCT`, so an actor nobody matches
    // is not in it; this drives the endpoint the Filter button would call.
    let path = world.path().await?;
    let base = path.split('?').next().unwrap_or("/admin").to_owned();
    world.goto(&format!("{base}?aactor={actor}")).await
}

#[then(expr = "every audit row shows the action {string}")]
async fn every_audit_row_shows(world: &mut PingwardWorld, action: String) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_visible("audit-row").await?;
    for row in driver.test_ids("audit-row").await? {
        let text = row.normalized_text().await?;
        ensure!(text.contains(&action), "an audit row reads {text:?}");
    }
    Ok(())
}

#[then("the audit clear filter link is visible")]
async fn audit_clear_visible(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible("audit-clear").await
}

#[then("the audit clear filter link is not visible")]
async fn audit_clear_absent(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_absent("audit-clear").await
}

#[when("I clear the audit filter")]
async fn clear_audit_filter(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.click("audit-clear").await?;
    // Mirror of the filter step: the link disappearing is the swap signal.
    driver.expect_absent("audit-clear").await
}

#[then("the audit trail is empty with a filtered message")]
async fn audit_empty_filtered(world: &mut PingwardWorld) -> Result<()> {
    world
        .driver()?
        .expect_text("audit-empty", "No audit entries match the filter.")
        .await
}

#[then("the ping URL is withheld")]
async fn ping_url_withheld(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_visible("ping-url-withheld").await?;
    driver.expect_absent("ping-url").await?;
    // The usage help repeats the URL, so it is gone too.
    driver.expect_absent("ping-help").await
}

#[given("I reveal the ping URL")]
#[when("I reveal the ping URL")]
async fn reveal_ping_url(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.submit("reveal-ping-url").await?;
    driver.expect_visible("ping-url").await
}

#[given(expr = "I unlock admin actions with my password {string}")]
async fn unlock_admin_actions(world: &mut PingwardWorld, password: String) -> Result<()> {
    // Creating a user, resetting a password and granting admin hand out access
    // that outlives the session, so they sit behind `src/elevate.rs`. Removing
    // access — disabling, demoting, deleting — never needs it.
    world.goto("/admin/unlock").await?;
    let driver = world.driver()?;
    driver.fill("unlock-input", &password).await?;
    driver.submit("unlock-submit").await?;
    world.driver()?.expect_visible("elevation-flash").await
}

#[when("I follow the confirm link on the admin page")]
async fn follow_confirm_link(world: &mut PingwardWorld) -> Result<()> {
    // The server bounces a refused action to the interstitial; with JS it is
    // reached by the link on `/admin`, which is what this drives. The bounce
    // itself is asserted in `tests/admin_elevation.rs`.
    world.goto("/admin").await?;
    world.driver()?.submit("elevation-confirm-link").await?;
    world.expect_path("/admin/unlock").await
}

#[then("the confirmation page explains the requirement")]
async fn confirmation_page_explains(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_visible("unlock-gated").await?;
    driver.expect_visible("unlock-cancel").await?;
    driver.expect_visible("unlock-input").await
}

#[given("I lock admin actions")]
async fn lock_admin_actions(world: &mut PingwardWorld) -> Result<()> {
    // Elevation is per-session and dropped on sign-out, so re-signing in is how
    // a scenario returns to the locked state; there is no "lock now" control.
    sign_in(world, "admin", "correct horse battery").await?;
    world.goto("/admin").await?;
    // Locked, `/admin` shows only a note linking to the interstitial.
    world
        .driver()?
        .expect_visible("elevation-confirm-link")
        .await
}

#[when(expr = "I fill in the new user {string} with password {string}")]
async fn fill_in_new_user(
    world: &mut PingwardWorld,
    username: String,
    password: String,
) -> Result<()> {
    world.goto("/admin").await?;
    let driver = world.driver()?;
    driver.fill("user-username-input", &username).await?;
    driver.fill("user-password-input", &password).await
}

#[when("I submit the new user form")]
async fn submit_new_user_form(world: &mut PingwardWorld) -> Result<()> {
    // Not `submit`: while locked, `app.js` intercepts the click and opens the
    // dialog instead of navigating.
    let driver = world.driver()?;
    let button = driver.test_id("user-submit").await?;
    click_when_ready(&button).await
}

#[then(expr = "the confirmation dialog appears naming {string}")]
async fn reauth_dialog_names(world: &mut PingwardWorld, action: String) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_visible("reauth-dialog").await?;
    driver.expect_exact_text("reauth-action", &action).await?;
    // Without this an admin goes hunting for an authenticator app.
    driver
        .expect_text("reauth-why", "not a second factor")
        .await
}

#[when(expr = "I confirm the dialog with password {string}")]
async fn confirm_dialog(world: &mut PingwardWorld, password: String) -> Result<()> {
    let driver = world.driver()?;
    driver.fill("reauth-input", &password).await?;
    driver.submit("reauth-submit").await
}

#[when(expr = "I answer the dialog with the wrong password {string}")]
async fn answer_dialog_wrongly(world: &mut PingwardWorld, password: String) -> Result<()> {
    let driver = world.driver()?;
    driver.fill("reauth-input", &password).await?;
    let button = driver.test_id("reauth-submit").await?;
    click_when_ready(&button).await?;
    driver.expect_visible("reauth-error").await
}

#[then("the dialog is still open with an error")]
async fn dialog_still_open(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_visible("reauth-dialog").await?;
    driver
        .expect_exact_text("reauth-error", "That password is not correct.")
        .await
}

#[when("I dismiss the dialog")]
async fn dismiss_dialog(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    let cancel = driver.test_id("reauth-cancel").await?;
    click_when_ready(&cancel).await?;
    driver.expect_hidden("reauth-dialog").await
}

#[then(expr = "the new user form still holds {string}")]
async fn new_user_form_still_holds(world: &mut PingwardWorld, username: String) -> Result<()> {
    world
        .driver()?
        .expect_value("user-username-input", &username)
        .await
}

#[then("no confirmation dialog appears")]
async fn no_reauth_dialog(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_absent("reauth-dialog").await
}
