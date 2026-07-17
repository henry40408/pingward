import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Given, When, Then } = createBdd(test);

// The applied theme is the data-theme attribute on <html>; it is always
// resolved to "light" or "dark" (never "system").
Then("the resolved theme is {string}", async ({ page }, theme) => {
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
});

// The persisted preference is localStorage['pw-theme'] ('light'|'dark'|'system').
Then("the stored theme preference is {string}", async ({ page }, pref) => {
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem("pw-theme")))
    .toBe(pref);
});

When("I click the theme toggle", async ({ page }) => {
  await page.locator("#pw-theme-toggle").click();
});

// Seed an explicit preference and reload so the head script re-resolves it.
Given("I set the theme preference to {string}", async ({ page }, pref) => {
  await page.evaluate((p) => localStorage.setItem("pw-theme", p), pref);
  await page.reload();
});

// Emulate the OS colour-scheme preference; the page's matchMedia 'change'
// listener re-resolves the theme when the preference is 'system'.
When("the OS prefers dark", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
});

When("the OS prefers light", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
});
