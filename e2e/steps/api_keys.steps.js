import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { When, Then } = createBdd(test);

When("I open the API keys page", async ({ page, serverUrl }) => {
  await page.goto(`${serverUrl}/api-keys`);
});

When("I create an API key named {string}", async ({ page }, name) => {
  await page.getByTestId("api-key-name-input").fill(name);
  await page.getByTestId("api-key-submit").click();
});

Then("the new API key token is shown once", async ({ page }) => {
  const token = page.getByTestId("api-key-token");
  await expect(token).toBeVisible();
  await expect(token).toContainText(/^pw_[0-9a-f]{64}$/);
});

Then("the API keys list shows a key named {string}", async ({ page }, name) => {
  await expect(page.getByRole("cell", { name, exact: true })).toBeVisible();
});

// The revoke button triggers a confirm() dialog; auto-accept it.
When("I revoke the API key", async ({ page }) => {
  page.once("dialog", (d) => d.accept());
  await page.getByTestId("api-key-delete").first().click();
});

Then("no API keys remain", async ({ page }) => {
  await expect(page.getByTestId("api-keys-empty")).toBeVisible();
});
