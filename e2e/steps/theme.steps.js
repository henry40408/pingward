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

When("I hover the dashboard's primary action", async ({ page, serverUrl }) => {
  await page.goto(serverUrl);
  await page.locator(".btn-primary").first().hover();
});

// WCAG relative-luminance contrast between the hovered element's own text and
// background colour. `filter` is not folded into computed colours, so this
// measures exactly the declarations a specificity clash would break.
Then("its label contrasts with its background", async ({ page }) => {
  const ratio = await page.locator(".btn-primary").first().evaluate((el) => {
    const cs = getComputedStyle(el);
    // Chromium resolves `color-mix()` to `color(srgb r g b)` with 0–1 floats,
    // while plain declarations stay `rgb(r, g, b)` in 0–255. Normalize both,
    // or a mixed colour reads as near-black and fakes a passing contrast.
    const rgb = (s) => {
      const n = s.match(/[\d.]+/g).slice(0, 3).map(Number);
      return s.startsWith("color(") ? n.map((v) => v * 255) : n;
    };
    const lum = ([r, g, b]) =>
      [r, g, b]
        .map((v) => {
          v /= 255;
          return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);
        })
        .reduce((acc, v, i) => acc + v * [0.2126, 0.7152, 0.0722][i], 0);
    const a = lum(rgb(cs.color));
    const b = lum(rgb(cs.backgroundColor));
    return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
  });
  expect(ratio).toBeGreaterThanOrEqual(4.5);
});
