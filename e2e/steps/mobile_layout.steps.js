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

// The Environment table is wider than a phone and scrolls inside .tscroll, so
// its cells must not wrap: a wrapping database URL made one row 331px tall, and
// a description column squeezed toward min-content made even "not set" rows
// ~195px. With both fixed every row is one or two lines (58px measured), so a
// row past 72px means one of the two has regressed.
Then("no Environment row is taller than 72px", async ({ page }) => {
  const tallest = await page.evaluate(() => {
    const rows = [...document.querySelectorAll('tr[data-testid^="env-row-"]')];
    return rows.reduce(
      (worst, r) => {
        const h = Math.round(r.getBoundingClientRect().height);
        return h > worst.h ? { h, id: r.dataset.testid } : worst;
      },
      { h: 0, id: "none" }
    );
  });
  expect(
    tallest.h,
    `${tallest.id} is ${tallest.h}px tall — its cells are wrapping`
  ).toBeLessThanOrEqual(72);
});

// The check page's breadcrumb links back to its project (templates/check.html).
When("I open the project from the breadcrumb", async ({ page }) => {
  await page.locator(".crumb a").nth(1).click();
  await expect(page).toHaveURL(/\/projects\/\d+$/);
});

// The reported symptom: .check hard-coded dashboard.html's child list, so
// project.html's extra child wrapped the badge onto a second grid row, which
// widened the auto-sized first column and stranded the 10px dot ~74px from the
// name. The row's own gap is 16px, so anything much beyond that is the bug.
Then("the check row's status dot sits next to the name", async ({ page }) => {
  const gap = await page.evaluate(() => {
    const row = document.querySelector(".check");
    const dot = row.querySelector(".status-dot").getBoundingClientRect();
    const meta = row.querySelector(".cmeta").getBoundingClientRect();
    return Math.round(meta.left - dot.right);
  });
  expect(gap, `.status-dot sits ${gap}px from .cmeta`).toBeLessThanOrEqual(20);
});

// The mechanism: when the badge wraps to another line its centre drops far
// below the dot's. On one line the two centres coincide.
Then("the check row is a single line", async ({ page }) => {
  const drop = await page.evaluate(() => {
    const row = document.querySelector(".check");
    const centre = (sel) => {
      const r = row.querySelector(sel).getBoundingClientRect();
      return r.top + r.height / 2;
    };
    return Math.round(Math.abs(centre(".badge") - centre(".status-dot")));
  });
  expect(drop, `.badge sits ${drop}px below the .status-dot — it wrapped`).toBeLessThanOrEqual(2);
});
