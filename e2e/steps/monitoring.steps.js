import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Given, When, Then } = createBdd(test);

// Navigate to the dashboard, then create a project via the "New project"
// flow. The project form lives at /projects/new and redirects to the project
// page on submit.
When("I create a project named {string}", async ({ page, serverUrl }, name) => {
  await page.goto(`${serverUrl}/projects/new`);
  await page.getByTestId("project-name-input").fill(name);
  await page.getByTestId("project-submit").click();
});

Then(
  "I am on the project page for {string}",
  async ({ page }, name) => {
    await expect(page).toHaveURL(/\/projects\/\d+$/);
    await expect(page.getByRole("heading", { name })).toBeVisible();
  }
);

// Create a project as a precondition and stay on its page (later steps read
// the "New check" link from it).
Given("a project named {string}", async ({ page, serverUrl }, name) => {
  await page.goto(`${serverUrl}/projects/new`);
  await page.getByTestId("project-name-input").fill(name);
  await page.getByTestId("project-submit").click();
  await expect(page).toHaveURL(/\/projects\/\d+$/);
});

// Create a check from the current project page. Period mode requires a
// positive period; grace/timezone are pre-filled by the form.
async function createCheck(page, name, period) {
  await page.getByTestId("new-check-link").click();
  await page.getByTestId("check-name-input").fill(name);
  await page.getByTestId("check-period-input").fill(String(period));
  await page.getByTestId("check-submit").click();
  await expect(page).toHaveURL(/\/checks\/\d+$/);
}

When(
  "I create a check named {string} with period {int}",
  async ({ page }, name, period) => {
    await createCheck(page, name, period);
  }
);

Given(
  "a check named {string} with period {int}",
  async ({ page }, name, period) => {
    await createCheck(page, name, period);
  }
);

Then("I am on the check page", async ({ page }) => {
  await expect(page).toHaveURL(/\/checks\/\d+$/);
});

// From the check page, follow the breadcrumb link back to its project. The
// breadcrumb's accessible name is the project name, and it's the only such
// link on the page.
When(
  "I visit the project page for {string}",
  async ({ page }, projectName) => {
    await page.getByRole("link", { name: projectName, exact: true }).click();
    await expect(page).toHaveURL(/\/projects\/\d+$/);
  }
);

Then("the check status is {string}", async ({ page }, status) => {
  await expect(page.getByTestId("check-status")).toHaveText(status);
});

Then("the check status is not {string}", async ({ page }, status) => {
  await expect(page.getByTestId("check-status")).not.toHaveText(status);
});

Then("the ping URL is shown", async ({ page }) => {
  await expect(page.getByTestId("ping-url")).toBeVisible();
});

// Read the URL the check page renders and drive a ping at it via the API
// helper. The page's rendered URL points at the test server because the
// harness sets PINGWARD_BASE_URL.
When("I send a {string} ping", async ({ page, api }, kind) => {
  const pingUrl = (await page.getByTestId("ping-url").textContent()).trim();
  await api.ping(pingUrl, kind);
});

When("I reload the check page", async ({ page }) => {
  await page.reload();
});

When("I acknowledge the check", async ({ page }) => {
  await page.getByTestId("ack-button").click();
});

Then("the acknowledge control is gone", async ({ page }) => {
  await expect(page.getByTestId("ack-button")).toHaveCount(0);
});

When("I pause the check", async ({ page }) => {
  await page.getByTestId("pause-button").click();
});

When("I resume the check", async ({ page }) => {
  await page.getByTestId("resume-button").click();
});

// Capture the current ping URL, regenerate, and confirm it changed.
When("I regenerate the ping URL", async ({ page }) => {
  const before = (await page.getByTestId("ping-url").textContent()).trim();
  await page.getByTestId("regenerate-button").click();
  await expect(page.getByTestId("ping-url")).not.toHaveText(before);
});

Then("the ping URL is different from before", async ({ page }) => {
  // The assertion is performed in the When step (the before-value is only in
  // scope there); here we simply confirm a ping URL is still present.
  await expect(page.getByTestId("ping-url")).toBeVisible();
});

// Delete flows submit through a confirm() dialog; accept it.
When("I delete the check", async ({ page }) => {
  page.on("dialog", (d) => d.accept());
  await page.getByTestId("delete-check-button").click();
  await expect(page).toHaveURL(/\/projects\/\d+$/);
});

Then("the project has no checks", async ({ page }) => {
  await expect(page.getByTestId("checks-empty")).toBeVisible();
});

When("I delete the project", async ({ page }) => {
  page.on("dialog", (d) => d.accept());
  await page.getByTestId("delete-project-button").click();
  await expect(page).toHaveURL(/\/$/);
});

Then("the dashboard shows no projects", async ({ page }) => {
  await expect(page.getByTestId("dashboard-empty")).toBeVisible();
});

Then("the recent pings table shows an empty state", async ({ page }) => {
  await expect(page.getByTestId("pings-empty")).toBeVisible();
});

Then("the recent notifications table shows an empty state", async ({ page }) => {
  await expect(page.getByTestId("notifications-empty")).toBeVisible();
});
