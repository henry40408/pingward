import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Given, When, Then } = createBdd(test);

// The new-project form lives at /projects/new.
Given("I open the new project form", async ({ page, serverUrl }) => {
  await page.goto(`${serverUrl}/projects/new`);
});

When("I fill the project name with {string}", async ({ page }, name) => {
  await page.getByTestId("project-name-input").fill(name);
});

// scan_interval_secs has no data-testid; select by id.
When(
  "I fill the project scan interval with {string}",
  async ({ page }, value) => {
    await page.locator("#scan_interval_secs").fill(value);
  }
);

When("I submit the project form", async ({ page }) => {
  await page.getByTestId("project-submit").click();
});

// Server-side validation re-renders the form with a flash error.
Then("the project form shows the error {string}", async ({ page }, message) => {
  await expect(page.locator(".flash.err")).toHaveText(message);
});

Then("the project name field shows {string}", async ({ page }, name) => {
  await expect(page.getByTestId("project-name-input")).toHaveValue(name);
});

// max_runtime_secs has no data-testid; select by id.
When("I fill the check max runtime with {string}", async ({ page }, value) => {
  await page.locator("#max_runtime_secs").fill(value);
});
