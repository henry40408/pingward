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

// Pager ends are always shown; reaching one disables (mutes, non-clickable via
// a rendered <span class="btn disabled">) rather than hiding its button.
Then("the pings {word} link is enabled", async ({ page }, dir) => {
  const link = page.getByTestId(`pings-${dir}`);
  await expect(link).toBeVisible();
  await expect(link).not.toHaveClass(/\bdisabled\b/);
});

Then("the pings {word} link is disabled", async ({ page }, dir) => {
  const link = page.getByTestId(`pings-${dir}`);
  await expect(link).toBeVisible();
  await expect(link).toHaveClass(/\bdisabled\b/);
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

When("I set the pings from date to {string}", async ({ page }, value) => {
  await page.getByTestId("pings-from").fill(value);
});

When("I apply the pings filter", async ({ page }) => {
  await page.getByTestId("pings-apply").click();
});

// The local wall-clock value round-trips through UTC and back, so the applied
// value matches what was entered regardless of the runner's time zone.
Then("the pings from date is {string}", async ({ page }, value) => {
  await expect(page.getByTestId("pings-from")).toHaveValue(value);
});

Then("the pings clear filter link is visible", async ({ page }) => {
  await expect(page.getByTestId("pings-clear")).toBeVisible();
});

Then("the pings clear filter link is not visible", async ({ page }) => {
  await expect(page.getByTestId("pings-clear")).toHaveCount(0);
});

// The two heartbeat invariants that only exist in CSS. Measured rather than
// asserted on markup: the bars are all rendered either way, and what this is
// checking is which of them the clipping box lets through.
Then(
  "the newest heartbeat bar is flush with the strip's right edge",
  async ({ page }) => {
    const gap = await page.evaluate(() => {
      const beat = document.querySelector(".beat");
      const bars = beat.querySelectorAll("i");
      const last = bars[bars.length - 1].getBoundingClientRect();
      return Math.round(beat.getBoundingClientRect().right - last.right);
    });
    expect(
      gap,
      `the newest run sits ${gap}px from the right edge — the strip is not right-aligned`
    ).toBeLessThanOrEqual(1);
  }
);

Then("the oldest heartbeat bars are clipped off the left", async ({ page }) => {
  // `scrollWidth` is no use here: the overflow runs off the *left* edge, which
  // it does not count. Compare what was rendered against what the clipping box
  // actually lets through instead.
  const m = await page.evaluate(() => {
    const beat = document.querySelector(".beat");
    const box = beat.getBoundingClientRect();
    const bars = [...beat.querySelectorAll("i")];
    return {
      rendered: bars.length,
      visible: bars.filter((b) => b.getBoundingClientRect().left >= box.left - 0.5).length,
      box: Math.round(box.width),
    };
  });
  expect(m.rendered).toBeGreaterThan(30);
  expect(
    m.visible,
    `all ${m.rendered} bars fit the ${m.box}px strip — nothing is being clipped, so this proves nothing`
  ).toBeLessThan(m.rendered);
});
