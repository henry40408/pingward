//! Projects, checks, pings and the dashboard — a port of
//! `monitoring.steps.js`.
//!
//! The project- and check-creating steps here are reused verbatim by
//! `admin.feature` after it navigates into `/admin/…`: the admin entity pages
//! are the owner templates with a different base prefix, so the same
//! `data-testid`s answer, and the URL assertions are deliberately unanchored
//! regexes that match both routes.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::PingKind;
use pingward_e2e::actions::{read_ping_url, reveal_ping_url_if_withheld};
use pingward_e2e::dom::{Dom, Within, click_when_ready};
use pingward_e2e::wait::eventually;
use pingward_e2e::world::PingwardWorld;

/// Matches both `/projects/{id}` and `/admin/projects/{id}`.
const PROJECT_PATH: &str = r"/projects/\d+$";

/// Matches both `/checks/{id}` and `/admin/checks/{id}`.
const CHECK_PATH: &str = r"/checks/\d+$";

#[given(expr = "I create a project named {string}")]
#[when(expr = "I create a project named {string}")]
async fn create_project(world: &mut PingwardWorld, name: String) -> Result<()> {
    open_new_project_form(world, &name).await
}

#[given(expr = "a project named {string}")]
async fn a_project_named(world: &mut PingwardWorld, name: String) -> Result<()> {
    open_new_project_form(world, &name).await?;
    world.expect_path_matching(PROJECT_PATH).await?;
    world.project_url = Some(world.path().await?);
    Ok(())
}

/// The "New project" flow: the form lives at `/projects/new` and redirects to
/// the project page on submit.
async fn open_new_project_form(world: &PingwardWorld, name: &str) -> Result<()> {
    world.goto("/projects/new").await?;
    let driver = world.driver()?;
    driver.fill("project-name-input", name).await?;
    driver.submit("project-submit").await
}

#[then(expr = "I am on the project page for {string}")]
async fn on_project_page_for(world: &mut PingwardWorld, name: String) -> Result<()> {
    world.expect_path_matching(PROJECT_PATH).await?;
    let heading = world.driver()?.heading_opt(&name).await?;
    ensure!(heading.is_some(), "no heading reads {name:?}");
    Ok(())
}

/// Creates a check from the current project page.
///
/// Period mode needs a positive period; grace and timezone are pre-filled by
/// the form.
async fn create_check(world: &PingwardWorld, name: &str, period: i64) -> Result<()> {
    let driver = world.driver()?;
    driver.submit("new-check-link").await?;
    driver.fill("check-name-input", name).await?;
    driver
        .fill("check-period-input", &period.to_string())
        .await?;
    driver.submit("check-submit").await?;
    world.expect_path_matching(CHECK_PATH).await
}

#[given(expr = "I create a check named {string} with period {int}")]
#[when(expr = "I create a check named {string} with period {int}")]
#[given(expr = "a check named {string} with period {int}")]
async fn a_check_named(world: &mut PingwardWorld, name: String, period: i64) -> Result<()> {
    create_check(world, &name, period).await
}

#[then("I am on the check page")]
async fn on_check_page(world: &mut PingwardWorld) -> Result<()> {
    world.expect_path_matching(CHECK_PATH).await
}

#[when(expr = "I visit the project page for {string}")]
async fn visit_project_page(world: &mut PingwardWorld, project: String) -> Result<()> {
    // From the check page, follow the breadcrumb back to its project. The
    // breadcrumb's accessible name is the project name, and it is the only
    // such link on the page.
    let body = world.driver()?.find(thirtyfour::By::Tag("body")).await?;
    let link = body
        .link_named_exact(&project)
        .await?
        .ok_or_else(|| anyhow::anyhow!("no link named {project:?}"))?;
    pingward_e2e::dom::submit_element(world.driver()?, &link).await?;
    world.expect_path_matching(PROJECT_PATH).await
}

#[then(expr = "the check status is {string}")]
async fn check_status_is(world: &mut PingwardWorld, status: String) -> Result<()> {
    world
        .driver()?
        .expect_exact_text("check-status", &status)
        .await
}

#[then(expr = "the check status is not {string}")]
async fn check_status_is_not(world: &mut PingwardWorld, status: String) -> Result<()> {
    world
        .driver()?
        .expect_not_exact_text("check-status", &status)
        .await
}

#[then("the ping URL is shown")]
async fn ping_url_shown(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible("ping-url").await
}

// Written under all three keywords across the suite: as setup, as the action
// under test, and — in `live_tail.feature` — as the thing whose arrival the
// assertion is about.
#[given(expr = "I send a {string} ping")]
#[when(expr = "I send a {string} ping")]
#[then(expr = "I send a {string} ping")]
async fn send_ping(world: &mut PingwardWorld, kind: String) -> Result<()> {
    // The page's rendered URL points at this scenario's server, because the
    // harness sets PINGWARD_BASE_URL to it.
    let ping_url = read_ping_url(world).await?;
    world
        .api()?
        .ping(&ping_url, PingKind::parse(&kind)?)
        .await?;
    world.ping_url = Some(ping_url);
    Ok(())
}

#[given("I reload the check page")]
#[when("I reload the check page")]
async fn reload_check_page(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.refresh().await?;
    Ok(())
}

#[when("I acknowledge the check")]
async fn acknowledge(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.submit("ack-button").await
}

#[then("the acknowledge control is gone")]
async fn ack_gone(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_absent("ack-button").await
}

#[given("I pause the check")]
#[when("I pause the check")]
async fn pause(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.submit("pause-button").await
}

#[when("I resume the check")]
async fn resume(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.submit("resume-button").await
}

#[when("I regenerate the ping URL")]
async fn regenerate_ping_url(world: &mut PingwardWorld) -> Result<()> {
    let before = read_ping_url(world).await?;
    world
        .driver()?
        .confirm_and_submit("regenerate-button")
        .await?;
    // Regenerating mints a *new* credential, and the redirect back lands on a
    // fresh render — so on the admin route it is withheld again until asked
    // for, which is the point: taking the new URL is its own disclosure.
    reveal_ping_url_if_withheld(world).await?;
    world
        .driver()?
        .expect_not_exact_text("ping-url", &before)
        .await
}

#[then("the ping URL is different from before")]
async fn ping_url_changed(world: &mut PingwardWorld) -> Result<()> {
    // The comparison happens in the `When` step, where the before-value is in
    // scope; here we confirm a ping URL is still present at all.
    world.driver()?.expect_visible("ping-url").await
}

#[when("I delete the check")]
async fn delete_check(world: &mut PingwardWorld) -> Result<()> {
    world
        .driver()?
        .confirm_and_submit("delete-check-button")
        .await?;
    world.expect_path_matching(PROJECT_PATH).await
}

#[then("the project has no checks")]
async fn project_has_no_checks(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible("checks-empty").await
}

#[when("I delete the project")]
async fn delete_project(world: &mut PingwardWorld) -> Result<()> {
    world
        .driver()?
        .confirm_and_submit("delete-project-button")
        .await?;
    world.expect_path_matching(r"/$").await
}

#[then("the dashboard shows no projects")]
async fn dashboard_empty(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible("dashboard-empty").await
}

#[when(expr = "I filter the dashboard by {string}")]
async fn filter_dashboard(world: &mut PingwardWorld, term: String) -> Result<()> {
    world.goto("/").await?;
    let driver = world.driver()?;
    driver.fill("dashboard-filter-input", &term).await?;
    driver.submit("dashboard-filter-submit").await?;
    // The filter is a plain GET form, so submitting it is a full navigation —
    // asserting the URL carries `q` proves the term round-tripped through the
    // server rather than being hidden client-side. The form also carries an
    // (empty) `status`, so `q` may be followed by `&status=` rather than
    // ending the query string.
    world
        .expect_path_matching(&format!(r"\?q={}(&|$)", regex::escape(&term)))
        .await
}

#[when(expr = "I filter the dashboard by status {string}")]
async fn filter_dashboard_by_status(world: &mut PingwardWorld, label: String) -> Result<()> {
    world.goto("/").await?;
    let driver = world.driver()?;
    driver
        .select_label("dashboard-status-filter", &label)
        .await?;
    driver.submit("dashboard-filter-submit").await?;
    // Asserting the URL carries the canonical `status=` value proves the
    // select round-tripped through the server.
    world
        .expect_path_matching(&format!("status={}(&|$)", label.to_lowercase()))
        .await
}

#[when("I clear the dashboard filter")]
async fn clear_dashboard_filter(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.submit("dashboard-filter-clear").await?;
    world.expect_path_matching(r"/$").await
}

#[when(expr = "I click the dashboard row for {string}")]
async fn click_dashboard_row(world: &mut PingwardWorld, name: String) -> Result<()> {
    world.goto("/").await?;
    let driver = world.driver()?;
    let rows = driver
        .test_ids_with_text("dashboard-check-row", &name)
        .await?;
    let row = rows
        .first()
        .ok_or_else(|| anyhow::anyhow!("no dashboard row for {name:?}"))?;
    // Deliberately the schedule line, not the name: the name is a real `<a>`
    // to the same place, so clicking it would prove the anchor works and leave
    // the delegated `data-href` handler untested.
    let schedule = row
        .css_opt(".sc")
        .await?
        .ok_or_else(|| anyhow::anyhow!("the row for {name:?} has no schedule line"))?;
    pingward_e2e::dom::submit_element(driver, &schedule).await
}

#[when(expr = "I click the row's edit link for {string}")]
async fn click_row_edit_link(world: &mut PingwardWorld, name: String) -> Result<()> {
    let driver = world.driver()?;
    let row = driver.css_row(".check", &name).await?;
    let link = row
        .link_named("edit")
        .await?
        .ok_or_else(|| anyhow::anyhow!("the row for {name:?} has no edit link"))?;
    click_when_ready(&link).await
}

#[then("I am on the check edit form")]
async fn on_check_edit_form(world: &mut PingwardWorld) -> Result<()> {
    world.expect_path_matching(r"/checks/\d+/edit$").await?;
    world.driver()?.expect_visible("check-name-input").await
}

#[then(expr = "the dashboard shows the check {string}")]
async fn dashboard_shows_check(world: &mut PingwardWorld, name: String) -> Result<()> {
    let driver = world.driver()?;
    eventually(&format!("the dashboard lists {name:?}"), || async {
        Ok(!driver
            .test_ids_with_text("dashboard-check-row", &name)
            .await?
            .is_empty())
    })
    .await
}

#[then(expr = "the dashboard does not show the check {string}")]
async fn dashboard_hides_check(world: &mut PingwardWorld, name: String) -> Result<()> {
    let driver = world.driver()?;
    eventually(&format!("the dashboard drops {name:?}"), || async {
        Ok(driver
            .test_ids_with_text("dashboard-check-row", &name)
            .await?
            .is_empty())
    })
    .await
}

#[then("the dashboard says nothing matched")]
async fn dashboard_no_results(world: &mut PingwardWorld) -> Result<()> {
    let driver = world.driver()?;
    driver.expect_visible("dashboard-no-results").await?;
    // "Nothing matched your filter" and "you have no projects at all" are
    // different statements; only one of them may be on the page.
    driver.expect_absent("dashboard-empty").await
}

#[then("the recent pings table shows an empty state")]
async fn pings_empty(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible("pings-empty").await
}

#[then("the recent notifications table shows an empty state")]
async fn notifications_empty(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.expect_visible("notifications-empty").await
}
