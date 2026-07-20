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
//
// On the signed-in admin's own row, the demote/disable/delete controls
// render as an inert `<span>` instead of a submitting `<form>` (see the
// "inert on your own row" scenario), so there is nothing to click. In that
// case `row` + `action` locate and drive the handler directly with a POST
// carrying the page's CSRF token: this still proves the handler's own
// self-guard refuses the action, independent of the UI hiding the control.
// The base path is read off the row's always-present, always-live
// password-reset form (`/admin/users/{id}/password`), which every other
// per-row action path shares.
async function submitRowAction(page, serverUrl, row, control, action) {
  if ((await control.evaluate((el) => el.tagName)) === "SPAN") {
    const resetAction = await row
      .locator('form[action$="/password"]')
      .getAttribute("action");
    const base = resetAction.replace(/\/password$/, "");
    const csrf = await page.locator('input[name="_csrf"]').first().inputValue();
    // The handler refuses with a 303 back to /admin. Assert that explicitly:
    // without it a 403 from `csrf_guard` (or an auth bounce) would leave the
    // state unchanged too, and the scenario would pass without ever reaching
    // the self-guard it exists to test. `page.request` follows redirects by
    // default, so `maxRedirects: 0` is required to see the 303 at all.
    const res = await page.request.post(`${serverUrl}${base}/${action}`, {
      form: { _csrf: csrf },
      maxRedirects: 0,
    });
    expect(res.status()).toBe(303);
    await page.goto(`${serverUrl}/admin`);
    return;
  }
  // Destructive controls (delete always; revoke-admin/disable when they'd
  // actually change state) now raise a confirm() dialog (see admin.html).
  // Playwright auto-dismisses dialogs by default, which would silently
  // block the form submit, so register a one-shot acceptor right before the
  // click. It's harmless on paths that raise no dialog ("make admin",
  // "enable") — the handler simply never fires.
  page.once("dialog", (d) => d.accept());
  await Promise.all([page.waitForNavigation({ waitUntil: "load" }), control.click()]);
}

// Fill the "Add user" form and submit; the handler redirects back to /admin.
// When `admin` is true the is_admin checkbox is checked, so the created user is
// an admin. The new row's visibility is awaited so the step only returns once
// the created user has actually rendered.
async function addUser(page, serverUrl, username, password, admin) {
  await page.getByTestId("user-username-input").fill(username);
  await page.getByTestId("user-password-input").fill(password);
  if (admin) await page.getByTestId("user-admin-checkbox").check();
  await Promise.all([
    page.waitForNavigation({ waitUntil: "load" }),
    page.getByTestId("user-submit").click(),
  ]);
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
  const row = userRow(page, username);
  await submitRowAction(page, serverUrl, row, row.getByTestId("user-toggle-admin"), "admin");
  await expect(page).toHaveURL(`${serverUrl}/admin`);
});

When("I disable {string}", async ({ page, serverUrl }, username) => {
  const row = userRow(page, username);
  await submitRowAction(
    page,
    serverUrl,
    row,
    row.getByTestId("user-toggle-disabled"),
    "disabled"
  );
  await expect(page).toHaveURL(`${serverUrl}/admin`);
});

When("I enable {string}", async ({ page, serverUrl }, username) => {
  const row = userRow(page, username);
  await submitRowAction(
    page,
    serverUrl,
    row,
    row.getByTestId("user-toggle-disabled"),
    "disabled"
  );
  await expect(page).toHaveURL(`${serverUrl}/admin`);
});

When(
  "I reset {string}'s password to {string}",
  async ({ page, serverUrl }, username, password) => {
    const row = userRow(page, username);
    await row.getByTestId("user-reset-input").fill(password);
    await Promise.all([
      page.waitForNavigation({ waitUntil: "load" }),
      row.getByTestId("user-reset-submit").click(),
    ]);
    await expect(page).toHaveURL(`${serverUrl}/admin`);
  }
);

When("I delete the user {string}", async ({ page, serverUrl }, username) => {
  const row = userRow(page, username);
  await submitRowAction(page, serverUrl, row, row.getByTestId("user-delete"), "delete");
  await expect(page).toHaveURL(`${serverUrl}/admin`);
});

// Unlike submitRowAction, this deliberately dismisses the confirm() dialog
// instead of accepting it, so the form never submits. Captures the dialog's
// message to prove the confirmation actually appeared (not just that the
// row survived, which a missing dialog would also produce).
When(
  "I attempt to delete {string} but dismiss the confirmation",
  async ({ page }, username) => {
    const row = userRow(page, username);
    let dialogMessage = null;
    page.once("dialog", async (d) => {
      dialogMessage = d.message();
      await d.dismiss();
    });
    await row.getByTestId("user-delete").click();
    await expect.poll(() => dialogMessage).toBe(
      "Delete this user? This cannot be undone."
    );
  }
);

// Gherkin action name -> the testid of its per-row control.
const SELF_ROW_TESTID = {
  demote: "user-toggle-admin",
  disable: "user-toggle-disabled",
  delete: "user-delete",
};

// The signed-in admin is always "admin" in this feature's Background.
Then("the {word} control on my own row is inert", async ({ page }, action) => {
  const control = userRow(page, "admin").getByTestId(SELF_ROW_TESTID[action]);
  expect(await control.evaluate((el) => el.tagName)).toBe("SPAN");
  await expect(control).toHaveClass(/\bdisabled\b/);
});

Then("the password reset control on my own row is usable", async ({ page }) => {
  const control = userRow(page, "admin").getByTestId("user-reset-submit");
  expect(await control.evaluate((el) => el.tagName)).toBe("BUTTON");
  await expect(control).toBeEnabled();
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
