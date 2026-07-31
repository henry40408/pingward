import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Given, When, Then } = createBdd(test);

When("I open the account page", async ({ page, serverUrl }) => {
  await page.goto(`${serverUrl}/account`);
});

Then("the current session is marked as this device", async ({ page }) => {
  await expect(page.getByTestId("session-current")).toBeVisible();
});

// The revoke button triggers a confirm() dialog; auto-accept it.
When("I revoke the current session", async ({ page }) => {
  page.once("dialog", (d) => d.accept());
  await page.getByTestId("session-revoke").first().click();
});

// --- password ---

When(
  "I change my password from {string} to {string}",
  async ({ page }, current, next) => {
    await page.getByTestId("current-password-input").fill(current);
    await page.getByTestId("new-password-input").fill(next);
    await page.getByTestId("confirm-password-input").fill(next);
    await page.getByTestId("password-submit").click();
  }
);

Then("the password change is confirmed", async ({ page }) => {
  await expect(page.getByTestId("password-changed-flash")).toBeVisible();
});

Then("the password change is rejected", async ({ page }) => {
  await expect(page.getByTestId("password-error")).toBeVisible();
  await expect(page.getByTestId("password-changed-flash")).toHaveCount(0);
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

// --- client IP behind a reverse proxy ---

// Every request from here on carries the header a proxy would add. Paired with
// the @trusted-proxy tag, which is what makes the server trust it.
Given("requests arrive through a trusted proxy as {string}", async ({ page }, ip) => {
  await page.setExtraHTTPHeaders({ "x-forwarded-for": ip });
});

// Covers the wiring the auth::client_ip unit tests cannot: that the login
// handler actually calls it and stores the result. Without this, reverting the
// call site to the raw socket peer would leave every test passing.
Then("the current session shows the IP {string}", async ({ page }, ip) => {
  const row = page.locator("tr", { has: page.getByTestId("session-current") });
  await expect(row).toContainText(ip);
});
