import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Given, When, Then } = createBdd(test);

// Open the new-check form from the current project page. The project's "New
// check" link (data-testid) leads to /projects/{pid}/checks/new.
async function openNewCheckForm(page) {
  await page.getByTestId("new-check-link").click();
  await expect(page).toHaveURL(/\/projects\/\d+\/checks\/new$/);
}

Given("I open the new check form", async ({ page }) => {
  await openNewCheckForm(page);
});

// Cron mode: pick the "cron" schedule kind and supply a 6-field expression.
// period_secs is left blank (ignored in cron mode). On success the handler
// redirects to the check detail page.
When(
  "I create a cron check named {string} with expression {string}",
  async ({ page }, name, expr) => {
    await openNewCheckForm(page);
    await page.getByTestId("check-name-input").fill(name);
    await page.locator("#schedule_kind").selectOption("cron");
    await page.locator("#cron_expr").fill(expr);
    await page.getByTestId("check-submit").click();
    await expect(page).toHaveURL(/\/checks\/\d+$/);
  }
);

When("I fill the check name with {string}", async ({ page }, name) => {
  await page.getByTestId("check-name-input").fill(name);
});

When("I fill the check period with {int}", async ({ page }, period) => {
  await page.getByTestId("check-period-input").fill(String(period));
});

// schedule_kind is a plain <select> with no data-testid; select by its id.
When("I choose the {string} schedule kind", async ({ page }, kind) => {
  await page.locator("#schedule_kind").selectOption(kind);
});

When("I submit the check form", async ({ page }) => {
  await page.getByTestId("check-submit").click();
});

// The schedule label renders on the check page; for a cron check it is the raw
// expression. Assert that expression text is shown.
Then("the check schedule shows {string}", async ({ page }, text) => {
  await expect(page.getByText(text)).toBeVisible();
});

// Submitting with an empty name is blocked client-side by the input's `required`
// attribute, so no POST fires and we stay on the form.
Then("I am still on the new check form", async ({ page }) => {
  await expect(page).toHaveURL(/\/projects\/\d+\/checks\/new$/);
  await expect(page.getByTestId("check-submit")).toBeVisible();
});

Then("the check name field is required", async ({ page }) => {
  await expect(page.getByTestId("check-name-input")).toHaveJSProperty(
    "validity.valueMissing",
    true
  );
});

// Server-side validation failures re-render the form with a flash error.
Then("the check form shows the error {string}", async ({ page }, message) => {
  await expect(page.locator(".flash.err")).toHaveText(message);
});
