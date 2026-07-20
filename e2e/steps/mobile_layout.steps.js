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

// A card body sets its own overflow-x, so a table placed directly in one makes
// the whole body scroll — dragging the Add-user form below it off-screen too
// (its left edge went 41px -> -20px). Wrapping the table in .tscroll confines
// the overflow to the table, leaving the body itself unscrollable.
Then("only the users table scrolls sideways, not the card around it", async ({ page }) => {
  const m = await page.evaluate(() => {
    const cb = document.querySelector('[data-testid="user-submit"]').closest(".cb");
    const table = cb.querySelector("table");
    return {
      bodyOverflow: cb.scrollWidth - cb.clientWidth,
      wrapper: table.parentElement.className,
    };
  });
  // Assert the symptom first so a failure names it; the wrapper check that
  // follows is a diagnostic pointing at the usual cause.
  expect(
    m.bodyOverflow,
    `the card body itself scrolls by ${m.bodyOverflow}px, so the Add-user form moves with the table`
  ).toBeLessThanOrEqual(0);
  expect(m.wrapper, "the users table is not wrapped in .tscroll").toContain("tscroll");
});

// The Environment table is wider than a phone and scrolls inside .tscroll, so
// wrapping its cells buys nothing and costs a lot of height: a breakable
// database URL made one row 331px tall, and a description column squeezed
// toward min-content made even "not set" rows ~195px.
//
// Two assertions, because they fail for different reasons. The value being one
// line box is exact and font-independent. The height bound is not — the same
// text measured 58px on macOS and 78px in Linux CI — so it is set from the
// defect side (195px+) rather than the fixed side, leaving room for whatever a
// third platform's metrics do.
Then("Environment rows do not wrap", async ({ page }) => {
  const m = await page.evaluate(() => {
    const rows = [...document.querySelectorAll('tr[data-testid^="env-row-"]')];
    const tallest = rows.reduce(
      (worst, r) => {
        const h = Math.round(r.getBoundingClientRect().height);
        return h > worst.h ? { h, id: r.dataset.testid } : worst;
      },
      { h: 0, id: "none" }
    );
    // The database URL is the longest value and has no spaces to break on.
    const code = document
      .querySelector('tr[data-testid="env-row-DATABASE_URL"]')
      .querySelector("code");
    return { tallest, valueLines: code.getClientRects().length };
  });
  expect(
    m.valueLines,
    `the DATABASE_URL value spans ${m.valueLines} lines — it is wrapping`
  ).toBe(1);
  expect(
    m.tallest.h,
    `${m.tallest.id} is ${m.tallest.h}px tall — the description column is being squeezed`
  ).toBeLessThanOrEqual(120);
});

// The three admin health tables (down checks, per-channel failures, recent
// failures) only render once there is failing data, and delivery runs on a
// background tokio::spawn with a retry policy, so the failure notification
// that populates channel_fail/recent_fail can land after this page load.
// Poll by reloading until all three are visible, so a failure in the next
// step's overflow assertion can never be misread as "the table wasn't there".
const HEALTH_TABLE_IDS = ["health-down", "health-channels", "health-recent"];

Then("the admin health tables are shown", async ({ page }) => {
  await expect(async () => {
    await page.reload();
    for (const id of HEALTH_TABLE_IDS) {
      await expect(page.getByTestId(id)).toBeVisible();
    }
  }).toPass({ timeout: 20000 });
});

// Same containment check as the users table above, generalized to all three
// admin health tables: the table's wrapper must be .tscroll, the wrapper must
// actually overflow (otherwise containment is vacuously satisfied by content
// that fits), and the card body itself must not scroll.
//
// The users-table step asserts the symptom (a scrolling card body) first so a
// failure names it. Here the order is inverted, because the two Notification
// health tables SHARE one .cb: unwrapping either drags that one body sideways,
// so the symptom cannot say which table caused it. Checking each table's own
// wrapper first pins the blame on the right table; the shared-body assertion
// then runs last, once every wrapper has been accounted for.
Then(
  "each admin health table scrolls inside its card, not the card around it",
  async ({ page }) => {
    const results = await page.evaluate((ids) => {
      return ids.map((id) => {
        const table = document.querySelector(`[data-testid="${id}"]`);
        const wrapper = table.parentElement;
        const cb = table.closest(".cb");
        return {
          id,
          wrapperClass: wrapper.className,
          bodyOverflow: cb.scrollWidth - cb.clientWidth,
          wrapperOverflow: wrapper.scrollWidth - wrapper.clientWidth,
        };
      });
    }, HEALTH_TABLE_IDS);

    for (const r of results) {
      expect(
        r.wrapperClass,
        `${r.id}: table is not wrapped in .tscroll (wrapper class: "${r.wrapperClass}")`
      ).toContain("tscroll");
      expect(
        r.wrapperOverflow,
        `${r.id}: the table's own wrapper does not overflow (${r.wrapperOverflow}px) — the seeded content is not wide enough to prove containment`
      ).toBeGreaterThan(0);
    }
    for (const r of results) {
      expect(
        r.bodyOverflow,
        `${r.id}: the card body itself scrolls by ${r.bodyOverflow}px, dragging sibling content with it`
      ).toBeLessThanOrEqual(0);
    }
  }
);

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
