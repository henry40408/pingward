import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { When, Then } = createBdd(test);

// exitcode pings hit /ping/{uuid}/{code}; code 0 -> up, non-zero -> down.
When(
  "I send an exit code {int} ping",
  async ({ page, api }, code) => {
    const pingUrl = (await page.getByTestId("ping-url").textContent()).trim();
    await api.ping(pingUrl, "exitcode", { code });
  }
);

// POST-body capture: the server records up to 10KB of the request body and
// renders it on the check page.
When(
  "I send a {string} ping with body {string}",
  async ({ page, api }, kind, body) => {
    const pingUrl = (await page.getByTestId("ping-url").textContent()).trim();
    await api.ping(pingUrl, kind, { method: "POST", body });
  }
);

// Unknown uuids 404 without ever hitting a check page, so drive the request
// directly against serverUrl and stash the status on world for the Then step.
When("I ping an unknown UUID", async ({ serverUrl, api, world }) => {
  world.pingStatus = await api.pingStatus(
    `${serverUrl}/ping/00000000-0000-0000-0000-000000000000`
  );
});

Then("the ping response status is {int}", async ({ world }, status) => {
  expect(world.pingStatus).toBe(status);
});

// The collapsible "How do I ping this check?" help documents every ping signal.
// Its content is present but hidden until the summary is clicked, so expand it
// before asserting the fail/start endpoints are shown.
Then(
  "the ping help documents the fail and start endpoints",
  async ({ page }) => {
    const help = page.getByTestId("ping-help");
    await help.locator("summary").click();
    await expect(help).toContainText("/fail");
    await expect(help).toContainText("/start");
  }
);

// Recent-pings kind cell renders as .pill.{class}; map the Gherkin kind label
// to its pill class. Scope to #pings-section: .badge is the status badge at the
// top, and the "How do I ping" help also uses .pill for its endpoint legend.
const PILL_CLASS = { success: "ok", fail: "fail", start: "start", log: "log" };

Then(
  "the recent pings table shows a {string} ping",
  async ({ page }, kind) => {
    const cls = PILL_CLASS[kind];
    await expect(
      page.locator(`#pings-section .pill.${cls}`).first()
    ).toBeVisible();
  }
);

Then(
  "the recent pings table shows the exit {string}",
  async ({ page }, exit) => {
    await expect(page.getByText(exit, { exact: true }).first()).toBeVisible();
  }
);

// Recent pings render newest-first, so the first tr.toggle is the row we just
// created. Only rows with a non-empty body render as toggle rows.
When("I expand the latest ping row", async ({ page }) => {
  await page.locator("tr.toggle").first().click();
});

Then("the captured output shows {string}", async ({ page }, text) => {
  // Scope to the pings section: the ping-help card also renders a .out block.
  const out = page.locator("#pings-section .out").first();
  await expect(out).toBeVisible();
  await expect(out).toContainText(text);
});
