import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { When, Then } = createBdd(test);

// These two scenarios in live_tail.feature are a deliberate pair: the
// "without the live tail" scenario is the control, proving via its final
// reload that the ping really was recorded — so the live-tail scenario
// passing can only be attributed to the live tail itself, never to a ping
// that silently failed to register.

// The backend only publishes an SSE "changed" event when
// events.receiver_count() > 0 (see ARCHITECTURE.md's Live-tail signal bus).
// A ping sent before the EventSource connection is actually open is dropped
// with no later signal to catch up on, so this step must wait for
// data-live="open" before returning — without it, sending the ping right
// after the click would be racy and intermittently fail.
When("I turn on the live tail", async ({ page }) => {
  await page.getByTestId("pings-live").click();
  await expect(page.getByTestId("pings-live")).toHaveAttribute(
    "data-live",
    "open"
  );
});

Then("the recent pings table still shows no pings", async ({ page }) => {
  await expect(page.getByTestId("pings-empty")).toBeVisible();
});
