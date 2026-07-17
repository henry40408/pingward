import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { When, Then } = createBdd(test);

// Create a period check that falls due almost immediately: period 1s, grace 0,
// so its due_time is ~1s after creation and the @fast-scan (1s) scan loop downs
// it without any ping. #grace_secs has no data-testid (id only) and the form
// pre-fills it to 300, so overwrite it.
When("I create a check that falls due almost immediately", async ({ page }) => {
  await page.getByTestId("new-check-link").click();
  await page.getByTestId("check-name-input").fill("overdue-job");
  await page.getByTestId("check-period-input").fill("1");
  await page.locator("#grace_secs").fill("0");
  await page.getByTestId("check-submit").click();
  await expect(page).toHaveURL(/\/checks\/\d+$/);
});

// Create a check with a long period (never overdue during the test) but a 1s
// max runtime. A later "start" ping opens an in-flight run whose deadline is
// start+1s; the scan loop then downs it via the overrun path only.
When("I create a check with a 1 second max runtime", async ({ page }) => {
  await page.getByTestId("new-check-link").click();
  await page.getByTestId("check-name-input").fill("overrun-job");
  await page.getByTestId("check-period-input").fill("3600");
  await page.locator("#grace_secs").fill("0");
  await page.locator("#max_runtime_secs").fill("1");
  await page.getByTestId("check-submit").click();
  await expect(page).toHaveURL(/\/checks\/\d+$/);
});

// The scan loop runs asynchronously in the pingward process, so poll by
// reloading the check page until the status badge reads "down".
Then("the check status eventually becomes down", async ({ page }) => {
  await expect(async () => {
    await page.reload();
    await expect(page.getByTestId("check-status")).toHaveText("down");
  }).toPass({ timeout: 15000 });
});
