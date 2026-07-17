import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { When, Then } = createBdd(test);

// The project page has a single uppercase "Edit" button; a per-check-row
// lowercase "edit" link only appears when the project has checks. `exact:
// true` avoids the lowercase link matching this locator.
When("I open the project edit form", async ({ page }) => {
  await page.getByRole("link", { name: "Edit", exact: true }).click();
  await expect(page).toHaveURL(/\/projects\/\d+\/edit$/);
});

When("I open the check edit form", async ({ page }) => {
  await page.getByRole("link", { name: "Edit", exact: true }).click();
  await expect(page).toHaveURL(/\/checks\/\d+\/edit$/);
});

When("I change the project name to {string}", async ({ page }, name) => {
  await page.getByTestId("project-name-input").fill(name);
  await page.getByTestId("project-submit").click();
});

When("I change the check name to {string}", async ({ page }, name) => {
  await page.getByTestId("check-name-input").fill(name);
  await page.getByTestId("check-submit").click();
});

When("I change the check period to {int}", async ({ page }, period) => {
  await page.getByTestId("check-period-input").fill(String(period));
  await page.getByTestId("check-submit").click();
});

// grace_secs has no data-testid; select by its id.
When("I change the check grace to {int}", async ({ page }, grace) => {
  await page.locator("#grace_secs").fill(String(grace));
  await page.getByTestId("check-submit").click();
});

// timezone has no data-testid; select by its id.
When("I change the check timezone to {string}", async ({ page }, tz) => {
  await page.locator("#timezone").fill(tz);
  await page.getByTestId("check-submit").click();
});

Then("the check name is {string}", async ({ page }, name) => {
  await expect(page.getByRole("heading", { name })).toBeVisible();
});

// The check page has no timezone display, so persistence is verified by
// reopening the edit form and reading the pre-filled value back out.
Then("the check timezone field shows {string}", async ({ page }, tz) => {
  await expect(page.locator("#timezone")).toHaveValue(tz);
});

Then("the check period field shows {string}", async ({ page }, period) => {
  await expect(page.getByTestId("check-period-input")).toHaveValue(period);
});
