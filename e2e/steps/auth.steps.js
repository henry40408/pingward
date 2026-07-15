import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { When, Then } = createBdd(test);

When("I visit {string}", async ({ page, serverUrl }, urlPath) => {
  await page.goto(`${serverUrl}${urlPath}`);
});

Then("I am on the setup page", async ({ page, serverUrl }) => {
  await expect(page).toHaveURL(`${serverUrl}/setup`);
  await expect(page.getByRole("button", { name: "Create admin" })).toBeVisible();
});

When(
  "I create the admin {string} with password {string}",
  async ({ page }, username, password) => {
    await page.locator("#username").fill(username);
    await page.locator("#password").fill(password);
    await page.getByRole("button", { name: "Create admin" }).click();
  }
);

Then("I land on the dashboard signed in", async ({ page, serverUrl }) => {
  await expect(page).toHaveURL(`${serverUrl}/`);
  await expect(page.getByRole("button", { name: "Log out" })).toBeVisible();
});
