import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";
import { signIn } from "../support/actions.js";

const { Given, When, Then } = createBdd(test);

// Seed a project + check OWNED BY a second (non-admin) user, so the later
// /admin/* scenarios exercise genuine cross-user access rather than the admin's
// own data. We sign in as the member (they must own the rows), create the
// project and check through the normal owner UI, and record their ids on
// `world` for the admin-area navigation steps. The member was already created
// in Background via the /users admin form. We arrive here already signed in as
// the admin; `signIn` handles the switch, since /login now bounces an
// authenticated visitor to "/" and has to be reached signed out.
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
// page anymore — both live as sections of the same page. The site-wide counts
// are the exception: they render as bare tiles with no heading of their own
// (like the dashboard's), so they are matched by data-testid.
Then("the admin dashboard is shown", async ({ page }) => {
  await expect(page.getByRole("heading", { name: "Admin", exact: true })).toBeVisible();
  await expect(page.getByTestId("admin-scale").locator(".tile")).toHaveCount(4);
});

// A .subhead sits inside a card body, so it must read as a level BELOW the
// card's own .ch h2, never above it. Compare computed font sizes rather than
// the declaration, since the bug was an inherited global h2 rather than
// anything written in the .subhead rule. The 2px allowance lets a subhead be
// nominally a touch larger than the uppercase, letter-spaced card label
// without reopening the 21px-vs-13px gap this guards.
Then("no card subheading renders larger than its card heading", async ({ page }) => {
  const m = await page.evaluate(() => {
    const size = (el) => parseFloat(getComputedStyle(el).fontSize);
    const heading = size(document.querySelector(".card .ch h2"));
    const subheads = [...document.querySelectorAll(".card .subhead")].map((el) => ({
      text: el.textContent.trim(),
      size: size(el),
    }));
    return { heading, subheads };
  });
  expect(m.subheads.length, "no .subhead rendered — the check would be vacuous").toBeGreaterThan(0);
  for (const s of m.subheads) {
    expect(
      s.size,
      `subheading "${s.text}" renders at ${s.size}px inside a ${m.heading}px card heading`
    ).toBeLessThanOrEqual(m.heading + 2);
  }
});

// `/admin/projects` no longer exists; the "All projects" section lives on
// `/admin`.
When("I open the admin projects list", async ({ page, serverUrl }) => {
  await page.goto(`${serverUrl}/admin`);
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

// Admin project delete redirects straight to /admin (the owner flow redirects
// to the dashboard "/"), so it needs its own step rather than reusing
// monitoring's.
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

// --- audit trail -----------------------------------------------------------
//
// The audit card is a swappable history section (`pw.wireSection` in
// base.html): its Filter button and pager links fetch `/admin/audit` and
// replace the card body in place, so every assertion below auto-waits for the
// swap rather than a navigation.

Then("the audit trail has at least {int} row", async ({ page }, n) => {
  await expect(page.getByTestId("audit-row").nth(n - 1)).toBeVisible();
});

Then("the audit trail shows an {string} entry", async ({ page }, action) => {
  await expect(
    page.getByTestId("audit-row").filter({ hasText: action }).first()
  ).toBeVisible();
});

// Each row with a request behind it is a `tr.toggle` followed by a `tr.exp`
// that gets `.open` on click — the ping table's captured-output pattern.
When("I expand the first audit row", async ({ page }) => {
  await page.getByTestId("audit-row").first().click();
});

Then("the audit detail shows the request path", async ({ page }) => {
  await expect(page.locator("#audit-section tr.exp.open").first()).toContainText(
    "/admin/"
  );
});

When("I filter the audit trail by action {string}", async ({ page }, action) => {
  await page.getByTestId("audit-action").selectOption(action);
  await page.getByTestId("audit-apply").click();
  // The Clear link is rendered only in a filtered response, so waiting for it
  // is what makes the following row assertions read the swapped-in table
  // rather than racing the still-present pre-filter rows.
  await expect(page.getByTestId("audit-clear")).toBeVisible();
});

When("I filter the audit trail by actor {string}", async ({ page }, actor) => {
  // An actor nobody matches is not in the select (it is built from the data),
  // so drive the endpoint the Filter button would call.
  await page.goto(`${page.url().split("?")[0]}?aactor=${actor}`);
});

Then("every audit row shows the action {string}", async ({ page }, action) => {
  const rows = page.getByTestId("audit-row");
  await expect(rows.first()).toBeVisible();
  for (const row of await rows.all()) {
    await expect(row).toContainText(action);
  }
});

Then("the audit clear filter link is visible", async ({ page }) => {
  await expect(page.getByTestId("audit-clear")).toBeVisible();
});

Then("the audit clear filter link is not visible", async ({ page }) => {
  await expect(page.getByTestId("audit-clear")).toHaveCount(0);
});

When("I clear the audit filter", async ({ page }) => {
  await page.getByTestId("audit-clear").click();
  // Mirror of the filter step: the Clear link disappearing is the swap signal.
  await expect(page.getByTestId("audit-clear")).toHaveCount(0);
});

Then("the audit trail is empty with a filtered message", async ({ page }) => {
  await expect(page.getByTestId("audit-empty")).toContainText(
    "No audit entries match the filter."
  );
});
