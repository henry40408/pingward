import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";
import { signIn } from "../support/actions.js";

const { Given, When, Then } = createBdd(test);

// Seed a project + check OWNED BY a second (non-admin) user, so the later
// /admin/* scenarios exercise genuine cross-user access rather than the admin's
// own data. We sign in as the member (they must own the rows), create the
// project and check through the normal owner UI, and record their ids on
// `world` for the admin-area navigation steps. The member was already created
// in Background via the /users admin form. No explicit logout is needed: the
// caller's "I am signed in as ..." step goes to /login (which always renders
// the form, never redirecting an authed user) and login_submit starts a fresh
// session that replaces the member's.
Given(
  "{string} with password {string} owns a project {string} with a check {string} period {int}",
  async ({ page, serverUrl, world }, username, password, projectName, checkName, period) => {
    await signIn(page, serverUrl, username, password);
    await expect(page).toHaveURL(`${serverUrl}/`);

    await page.goto(`${serverUrl}/projects/new`);
    await page.getByTestId("project-name-input").fill(projectName);
    await page.getByTestId("project-submit").click();
    await expect(page).toHaveURL(/\/projects\/\d+$/);
    world.projectId = page.url().match(/\/projects\/(\d+)$/)[1];

    await page.getByTestId("new-check-link").click();
    await page.getByTestId("check-name-input").fill(checkName);
    await page.getByTestId("check-period-input").fill(String(period));
    await page.getByTestId("check-submit").click();
    await expect(page).toHaveURL(/\/checks\/\d+$/);
    world.checkId = page.url().match(/\/checks\/(\d+)$/)[1];
  }
);

// --- admin-area navigation (direct goto by remembered id) ---

When("I open the admin dashboard", async ({ page, serverUrl }) => {
  await page.goto(`${serverUrl}/admin`);
});

// The merged /admin page carries no data-testid on its section headings, so we
// assert against heading text instead. There's no dedicated dashboard/projects
// page anymore — both live as sections of the same page.
Then("the admin dashboard is shown", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Admin", exact: true })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Scale" })).toBeVisible();
});

// `/admin/projects` is now a legacy redirect to `/admin`, where the "All
// projects" section lives.
When("I open the admin projects list", async ({ page, serverUrl }) => {
  await page.goto(`${serverUrl}/admin/projects`);
});

Then(
  "the admin projects list shows {string} owned by {string}",
  async ({ page }, projectName, owner) => {
    await expect(page.getByRole("heading", { name: "All projects" })).toBeVisible();
    const row = page.locator(".check", { hasText: projectName });
    await expect(row).toBeVisible();
    await expect(row).toContainText(`owner: ${owner}`);
  }
);

// Open the member's project/check under /admin. The rendered pages are the
// shared owner templates with /admin-prefixed forms, so downstream steps reuse
// the monitoring step definitions (pause/resume/ack/regenerate/ping/status).
When("I open the member's project in the admin area", async ({ page, serverUrl, world }) => {
  await page.goto(`${serverUrl}/admin/projects/${world.projectId}`);
});

When("I open the member's check in the admin area", async ({ page, serverUrl, world }) => {
  await page.goto(`${serverUrl}/admin/checks/${world.checkId}`);
});

// Both project.html and check.html render the entity name as the page <h1>.
Then("I am viewing the check {string}", async ({ page }, name) => {
  await expect(page.getByRole("heading", { name })).toBeVisible();
});

// --- admin cross-user mutations unique to the /admin surface ---

When("I rename the project to {string}", async ({ page, serverUrl, world }, name) => {
  await page.goto(`${serverUrl}/admin/projects/${world.projectId}/edit`);
  await page.getByTestId("project-name-input").fill(name);
  await page.getByTestId("project-submit").click();
});

Then(
  "I am on the admin project page for {string}",
  async ({ page, serverUrl, world }, name) => {
    await expect(page).toHaveURL(`${serverUrl}/admin/projects/${world.projectId}`);
    await expect(page.getByRole("heading", { name })).toBeVisible();
  }
);

// channel_form.html has no data-testid attributes; select by id/name. Webhook is
// the default kind, so only the name + webhook URL fields need filling.
When("I add a webhook channel named {string}", async ({ page, serverUrl, world }, name) => {
  await page.goto(`${serverUrl}/admin/projects/${world.projectId}/channels/new`);
  await page.locator("#name").fill(name);
  await page.locator("#webhook_url").fill("https://example.com/hook");
  await page.getByRole("button", { name: "Create channel" }).click();
  await expect(page).toHaveURL(`${serverUrl}/admin/projects/${world.projectId}`);
});

Then("the channel {string} is listed on the project", async ({ page }, name) => {
  await expect(page.locator(".chk .nm", { hasText: name })).toBeVisible();
});

// Admin project delete redirects to /admin/projects (the owner flow redirects
// to the dashboard "/"), so it needs its own step rather than reusing
// monitoring's. /admin/projects is itself now a legacy redirect to /admin, so
// the final landing page is /admin.
When("I delete the member's project", async ({ page, serverUrl }) => {
  page.on("dialog", (d) => d.accept());
  await page.getByTestId("delete-project-button").click();
  await expect(page).toHaveURL(`${serverUrl}/admin`);
});

Then("the admin projects list has no projects", async ({ page }) => {
  await expect(page.getByText("No projects yet.")).toBeVisible();
});

// --- Environment card (read-only env-var settings on /admin) ---

Then("the Environment card shows the SMTP password as configured", async ({ page }) => {
  await expect(page.getByTestId("env-smtp-password")).toContainText("configured");
});

Then("the page does not contain the SMTP secret", async ({ page }) => {
  await expect(page.locator("body")).not.toContainText("e2e-secret-password");
});
