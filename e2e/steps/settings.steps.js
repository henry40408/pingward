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
