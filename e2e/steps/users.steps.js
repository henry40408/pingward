import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Given, When, Then } = createBdd(test);

// Locate a user's <tr> by its per-row data-testid (user-row-<username>). Every
// control (reset/toggle-admin/toggle-disabled/delete) and status pill carries a
// row-local testid, so scoping to the row keeps selectors unambiguous even when
// a username collides with another row's role-pill text ("member").
const userRow = (page, username) => page.getByTestId(`user-row-${username}`);

// Every mutating control on /admin POSTs a form that redirects back to /admin.
// Because the URL is unchanged, `toHaveURL('/admin')` would resolve instantly
// without waiting for the redirect to commit — leaving assertions to read the
// stale pre-navigation DOM (a false pass for the "state unchanged" guard
// scenarios) and risking the next step's navigation aborting an in-flight POST.
// Awaiting the navigation ties the step to the re-rendered page.
async function submitRowAction(page, locator) {
  await Promise.all([page.waitForNavigation({ waitUntil: "load" }), locator.click()]);
}

// Fill the "Add user" form and submit; the handler redirects back to /admin.
// When `admin` is true the is_admin checkbox is checked, so the created user is
// an admin. The new row's visibility is awaited so the step only returns once
// the created user has actually rendered.
async function addUser(page, serverUrl, username, password, admin) {
  await page.getByTestId("user-username-input").fill(username);
  await page.getByTestId("user-password-input").fill(password);
  if (admin) await page.getByTestId("user-admin-checkbox").check();
  await submitRowAction(page, page.getByTestId("user-submit"));
  await expect(page).toHaveURL(`${serverUrl}/admin`);
  await expect(userRow(page, username)).toBeVisible();
}

Given("I am on the users page", async ({ page, serverUrl }) => {
  await page.goto(`${serverUrl}/admin`);
});

When(
  "I add a user {string} with password {string}",
  async ({ page, serverUrl }, username, password) => {
    await addUser(page, serverUrl, username, password, false);
  }
);

When(
  "I add an admin user {string} with password {string}",
  async ({ page, serverUrl }, username, password) => {
    await addUser(page, serverUrl, username, password, true);
  }
);

// Preconditions: create a member (unchecked) or admin (checked) up front so a
// scenario can then act on the resulting row.
Given(
  "a member {string} with password {string} exists",
  async ({ page, serverUrl }, username, password) => {
    await addUser(page, serverUrl, username, password, false);
  }
);

Given(
  "an admin user {string} with password {string} exists",
  async ({ page, serverUrl }, username, password) => {
    await addUser(page, serverUrl, username, password, true);
  }
);

// Each mutating control submits a POST form that redirects to /admin, so after
// the click we wait for the reloaded page before the assertion runs.
When("I toggle admin on {string}", async ({ page, serverUrl }, username) => {
  await submitRowAction(page, userRow(page, username).getByTestId("user-toggle-admin"));
  await expect(page).toHaveURL(`${serverUrl}/admin`);
});

When("I disable {string}", async ({ page, serverUrl }, username) => {
  await submitRowAction(page, userRow(page, username).getByTestId("user-toggle-disabled"));
  await expect(page).toHaveURL(`${serverUrl}/admin`);
});

When("I enable {string}", async ({ page, serverUrl }, username) => {
  await submitRowAction(page, userRow(page, username).getByTestId("user-toggle-disabled"));
  await expect(page).toHaveURL(`${serverUrl}/admin`);
});

When(
  "I reset {string}'s password to {string}",
  async ({ page, serverUrl }, username, password) => {
    const row = userRow(page, username);
    await row.getByTestId("user-reset-input").fill(password);
    await submitRowAction(page, row.getByTestId("user-reset-submit"));
    await expect(page).toHaveURL(`${serverUrl}/admin`);
  }
);

When("I delete the user {string}", async ({ page, serverUrl }, username) => {
  await submitRowAction(page, userRow(page, username).getByTestId("user-delete"));
  await expect(page).toHaveURL(`${serverUrl}/admin`);
});

Then(
  "the user {string} is listed with role {string}",
  async ({ page }, username, role) => {
    await expect(userRow(page, username).getByTestId("user-role")).toHaveText(role);
  }
);

Then("the user {string} is marked disabled", async ({ page }, username) => {
  await expect(userRow(page, username).getByTestId("user-disabled")).toBeVisible();
});

Then("the user {string} is not marked disabled", async ({ page }, username) => {
  await expect(userRow(page, username)).toBeVisible();
  await expect(userRow(page, username).getByTestId("user-disabled")).toHaveCount(0);
});

Then("the user {string} is not listed", async ({ page }, username) => {
  await expect(userRow(page, username)).toHaveCount(0);
});
