import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Given, When, Then } = createBdd(test);

// Human nav label -> its data-testid on the admin-only nav links.
const NAV_TESTID = { Settings: "nav-settings", Admin: "nav-admin" };

// Create a non-admin user through the admin-only /users form. Assumes the
// admin is signed in (the form is guarded by AdminUser). The is_admin
// checkbox is left unchecked, so the created user is a non-admin. On success
// the handler redirects back to /users, where the new username is listed.
Given(
  "a non-admin user {string} with password {string} exists",
  async ({ page, serverUrl }, username, password) => {
    await page.goto(`${serverUrl}/users`);
    await page.getByTestId("user-username-input").fill(username);
    await page.getByTestId("user-password-input").fill(password);
    await page.getByTestId("user-submit").click();
    await expect(page).toHaveURL(`${serverUrl}/users`);
    await expect(page.getByText(username, { exact: true }).first()).toBeVisible();
  }
);

Then("the {string} nav link is visible", async ({ page }, label) => {
  await expect(page.getByTestId(NAV_TESTID[label])).toBeVisible();
});

Then("the {string} nav link is not visible", async ({ page }, label) => {
  await expect(page.getByTestId(NAV_TESTID[label])).toHaveCount(0);
});

// Navigate to a path and record the final response status on `world`.
When("I navigate to {string}", async ({ page, serverUrl, world }, path) => {
  const res = await page.goto(`${serverUrl}${path}`);
  world.status = res.status();
});

Then("the response status is {int}", async ({ world }, status) => {
  expect(world.status).toBe(status);
});

// Issue an authenticated POST with no _csrf field. page.request shares the
// browser context's cookies (the session cookie rides along), so this hits
// the CSRF guard as a real logged-in request that simply omits the token. The
// scenario asserts a live admin session first (the Admin nav link), so the 403
// is attributable to the missing token, not a missing session.
When(
  "I POST to {string} without a CSRF token",
  async ({ page, serverUrl, world }, path) => {
    const res = await page.request.post(`${serverUrl}${path}`, { form: {} });
    world.status = res.status();
  }
);

// Remember the current page URL (a project page) for a later cross-user visit.
// Wait for the project URL first: the monitoring "I create a project named"
// step clicks submit without awaiting the redirect, so page.url() could still
// read /projects/new if we captured it immediately.
When("I remember the current project", async ({ page, world }) => {
  await expect(page).toHaveURL(/\/projects\/\d+$/);
  world.projectUrl = page.url();
});

// Positive control for the cross-user 404: the owner (currently signed in)
// reads the remembered project and gets 200, so the later 404 for a different
// user is attributable to the ownership guard rather than a broken/missing
// route or a non-existent project.
Then("the owner can read the remembered project", async ({ page, world }) => {
  const res = await page.goto(world.projectUrl);
  expect(res.status()).toBe(200);
});

// Sign out, sign in as another user, revisit the remembered project URL, and
// record the response status (expected 404: the project exists but is owned by
// someone else, and owned_project hides existence).
When(
  "I revisit it as {string} with password {string}",
  async ({ page, serverUrl, world }, username, password) => {
    await page.getByTestId("logout-button").click();
    await page.goto(`${serverUrl}/login`);
    await page.getByTestId("username-input").fill(username);
    await page.getByTestId("password-input").fill(password);
    await page.getByTestId("login-submit").click();
    await expect(page).toHaveURL(`${serverUrl}/`);
    const res = await page.goto(world.projectUrl);
    world.status = res.status();
  }
);
