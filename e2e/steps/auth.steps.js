import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Given, When, Then } = createBdd(test);

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

Given(
  "an admin {string} with password {string} exists",
  async ({ api }, username, password) => {
    await api.bootstrapAdmin(username, password);
  }
);

When(
  "I sign in as {string} with password {string}",
  async ({ page, serverUrl }, username, password) => {
    await page.goto(`${serverUrl}/login`);
    await page.locator("#username").fill(username);
    await page.locator("#password").fill(password);
    await page.getByRole("button", { name: "Log in" }).click();
  }
);

Then(
  "the login page shows the error {string}",
  async ({ page, serverUrl }, message) => {
    await expect(page).toHaveURL(`${serverUrl}/login`);
    await expect(page.getByText(message)).toBeVisible();
  }
);

Given(
  "I am signed in as {string} with password {string}",
  async ({ page, serverUrl }, username, password) => {
    await page.goto(`${serverUrl}/login`);
    await page.locator("#username").fill(username);
    await page.locator("#password").fill(password);
    await page.getByRole("button", { name: "Log in" }).click();
    await expect(page).toHaveURL(`${serverUrl}/`);
  }
);

When("I sign out", async ({ page }) => {
  await page.getByRole("button", { name: "Log out" }).click();
});

Then("I am on the login page", async ({ page, serverUrl }) => {
  await expect(page).toHaveURL(`${serverUrl}/login`);
  await expect(page.getByRole("button", { name: "Log in" })).toBeVisible();
});
