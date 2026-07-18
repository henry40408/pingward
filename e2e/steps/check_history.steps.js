import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { When, Then } = createBdd(test);

// Seeding pings is cheap over HTTP: each is a bare GET to the check's ping
// URL (no body), so it renders as a single plain (non-toggle) `tr` row —
// see the "recent pings" table markup in templates/check.html.
When("I send {int} {string} pings", async ({ page, api }, count, kind) => {
  const pingUrl = (await page.getByTestId("ping-url").textContent()).trim();
  for (let i = 0; i < count; i++) {
    await api.ping(pingUrl, kind);
  }
});

Then("the pings table shows {int} rows", async ({ page }, count) => {
  await expect(page.getByTestId("ping-row")).toHaveCount(count);
});

Then("the pings older link is visible", async ({ page }) => {
  await expect(page.getByTestId("pings-older")).toBeVisible();
});

Then("the pings older link is not visible", async ({ page }) => {
  await expect(page.getByTestId("pings-older")).toHaveCount(0);
});

Then("the pings newer link is visible", async ({ page }) => {
  await expect(page.getByTestId("pings-newer")).toBeVisible();
});

Then("the pings newer link is not visible", async ({ page }) => {
  await expect(page.getByTestId("pings-newer")).toHaveCount(0);
});

When("I click the pings older link", async ({ page }) => {
  await page.getByTestId("pings-older").click();
});

When("I click the pings newer link", async ({ page }) => {
  await page.getByTestId("pings-newer").click();
});

// Filtering swaps the pings section in place via a fetch to the fragment
// endpoint; the subsequent row-count assertion auto-waits for the swap.
When("I filter pings by kind {string}", async ({ page }, kind) => {
  await page.getByTestId("pings-kind").selectOption(kind);
  await page.getByTestId("pings-apply").click();
});

When("I clear the pings filter", async ({ page }) => {
  await page.getByTestId("pings-clear").click();
});

Then("the pings clear filter link is visible", async ({ page }) => {
  await expect(page.getByTestId("pings-clear")).toBeVisible();
});

Then("the pings clear filter link is not visible", async ({ page }) => {
  await expect(page.getByTestId("pings-clear")).toHaveCount(0);
});
