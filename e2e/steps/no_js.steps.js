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
