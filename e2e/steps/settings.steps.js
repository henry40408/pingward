import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { When, Then } = createBdd(test);

// Settings inputs have ids but no data-testid; drive them by id.
When(
  "I fill the settings field {string} with {string}",
  async ({ page }, field, value) => {
    await page.locator(`#${field}`).fill(value);
  }
);

// Saving POSTs to /settings. On success the handler 303-redirects back to
// /settings; on a validation error it re-renders /settings inline. Either way
// the URL stays /settings, so a bare toHaveURL would resolve against the stale
// pre-submit DOM (a false pass, since the just-typed values are still shown).
// Awaiting the navigation ties the step to the reloaded page.
When("I save the settings form", async ({ page }) => {
  await Promise.all([
    page.waitForNavigation({ waitUntil: "load" }),
    page.getByRole("button", { name: "Save changes" }).click(),
  ]);
});

Then(
  "the settings field {string} shows {string}",
  async ({ page }, field, value) => {
    await expect(page.locator(`#${field}`)).toHaveValue(value);
  }
);

Then("the settings form shows the error {string}", async ({ page }, message) => {
  await expect(page.locator(".flash.err")).toHaveText(message);
});

// After saving settings the page shows a one-shot success flash (backed by a
// flash cookie that is cleared on this render), mirroring the check page's
// notify-channels flash.
Then("the settings page shows the flash {string}", async ({ page }, message) => {
  await expect(page.getByTestId("settings-flash")).toHaveText(message);
});

// The flash is one-shot: a fresh render (a reload, or a rejected save that
// re-renders without ever setting the cookie) must not show it.
Then("the settings page shows no flash", async ({ page }) => {
  await expect(page.getByTestId("settings-flash")).toHaveCount(0);
});
