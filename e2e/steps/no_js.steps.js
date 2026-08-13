// Steps for `no_js.feature`, which runs in the scripting-disabled `no-js`
// project. Nothing here may reach for a `data-testid` that only exists after
// `app.js` has run — that is the whole point of the file.
//
// The two assertions about the ping-output panel are also used from
// `check_history.feature` with script *on*, where they assert the opposite:
// a fix that simply left every panel open would satisfy the no-JS scenario and
// break nothing else, so the collapse has to be pinned from both sides.
import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { When, Then } = createBdd(test);

// A finish ping carrying a body, which is what pingward captures and renders
// in the expandable panel. POST because that is how a body reaches it.
When(
  "I send a failing ping with output {string}",
  async ({ page, api }, output) => {
    const pingUrl = (await page.getByTestId("ping-url").textContent()).trim();
    await api.ping(pingUrl, "fail", { method: "POST", body: output });
  }
);

// `toBeVisible` rather than a text match: the output has always been *in* the
// DOM, hidden by `tr.exp { display: none }`. Being rendered is the claim.
Then("the captured output {string} is visible", async ({ page }, output) => {
  await expect(
    page.locator("#pings-section .out").filter({ hasText: output })
  ).toBeVisible();
});

Then("the captured output {string} is hidden", async ({ page }, output) => {
  await expect(
    page.locator("#pings-section .out").filter({ hasText: output })
  ).toBeHidden();
});

// Carets are drawn with `opacity`, not `display`, so the column keeps its
// width — which means "invisible" here is a computed-style assertion rather
// than `toBeHidden()`.
Then("the expand carets are invisible", async ({ page }) => {
  const opacities = await page
    .locator("#pings-section .caret")
    .evaluateAll((els) => els.map((el) => getComputedStyle(el).opacity));
  expect(opacities.length, "no carets on the page — assertion is vacuous").
    toBeGreaterThan(0);
  for (const o of opacities) expect(Number(o)).toBe(0);
});

When("I expand the first ping row", async ({ page }) => {
  await page.locator("#pings-section tr.toggle").first().click();
});

// The row's real anchor, not the delegated `data-href` handler.
When(
  "I click the dashboard check link for {string}",
  async ({ page }, name) => {
    await page
      .getByTestId("dashboard-check-row")
      .filter({ hasText: name })
      .getByRole("link", { name })
      .click();
  }
);

// A plain form submission: pick the value, press the button, follow the
// navigation. With script the same click is cancelled and swapped in place, so
// there is no shared step for this.
When("I filter the pings by kind {string}", async ({ page }, kind) => {
  await page.getByTestId("pings-kind").selectOption(kind);
  await page.getByTestId("pings-apply").click();
});

When(
  "I filter the notifications by event {string}",
  async ({ page }, event) => {
    await page.getByTestId("notifs-event").selectOption(event);
    await page.getByTestId("notifs-apply").click();
  }
);

// The selected value surviving a round trip is what proves the filter reached
// the server and came back rendered, rather than the page merely reloading.
Then("the pings kind filter shows {string}", async ({ page }, kind) => {
  await expect(page.getByTestId("pings-kind")).toHaveValue(kind);
});

Then(
  "the notifications event filter shows {string}",
  async ({ page }, event) => {
    await expect(page.getByTestId("notifs-event")).toHaveValue(event);
  }
);

// A CDP-level media override, so it works with scripting disabled — the page
// never has to be asked anything.
When("my system prefers {string}", async ({ page }, scheme) => {
  await page.emulateMedia({ colorScheme: scheme });
});

// Asserted by brightness rather than an exact token value: the claim is "this
// is a light page", and pinning `#f3f5f8` would turn any future palette tweak
// into a failure that says nothing about whether the theme still works.
async function backgroundLuminance(page) {
  return page.evaluate(() => {
    const [r, g, b] = getComputedStyle(document.body)
      .backgroundColor.match(/\d+(\.\d+)?/g)
      .map(Number);
    return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
  });
}

Then("the page background is light", async ({ page }) => {
  expect(await backgroundLuminance(page)).toBeGreaterThan(0.5);
});

Then("the page background is dark", async ({ page }) => {
  expect(await backgroundLuminance(page)).toBeLessThan(0.5);
});

// `toBeHidden`, not `toHaveCount(0)`: these are hidden by CSS, so the elements
// are still in the DOM and a count assertion would fail for the wrong reason —
// and would keep failing even if the rule worked.
Then("the copy button is absent", async ({ page }) => {
  await expect(page.locator(".copy").first()).toBeHidden();
});

Then("the live tail toggle is absent", async ({ page }) => {
  await expect(page.getByTestId("pings-live")).toBeHidden();
});

Then("the theme toggle is absent", async ({ page }) => {
  await expect(page.locator("#pw-theme-toggle")).toBeHidden();
});

Then("the scheduler heartbeat shows an age", async ({ page }) => {
  await expect(page.locator("[data-testid=sched-scan] .hb-ago")).toContainText(
    /\d+[smhd] ago/
  );
});

// Deliberately not monitoring.steps.js's "I delete the check": that one accepts
// a `confirm()` dialog and expects to land back on the project page, which is
// the scripted path. Here the click is expected to reach an interstitial.
When("I click the delete check button", async ({ page }) => {
  await page.getByTestId("delete-check-button").click();
});

Then("the confirmation page asks about deleting", async ({ page }) => {
  await expect(page.getByTestId("confirm-message")).toContainText("history");
  await expect(page.getByTestId("confirm-submit")).toBeVisible();
});

When("I confirm the pending action", async ({ page }) => {
  await page.getByTestId("confirm-submit").click();
});
