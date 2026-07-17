import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { When, Then } = createBdd(test);

When("I view the site at {int}px wide", async ({ page }, width) => {
  await page.setViewportSize({ width, height: 667 });
});

Then("the page has no horizontal scrollbar", async ({ page }) => {
  const { scrollW, clientW } = await page.evaluate(() => ({
    scrollW: document.documentElement.scrollWidth,
    clientW: document.documentElement.clientWidth,
  }));
  expect(
    scrollW,
    `page scrolls horizontally: scrollWidth ${scrollW}px > viewport ${clientW}px`
  ).toBeLessThanOrEqual(clientW);
});
