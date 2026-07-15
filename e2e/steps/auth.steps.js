import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { When, Then } = createBdd(test);

When("I visit {string}", async ({ page, serverUrl }, urlPath) => {
  await page.goto(`${serverUrl}${urlPath}`);
});

Then("I am on the setup page", async ({ page, serverUrl }) => {
  await expect(page).toHaveURL(`${serverUrl}/setup`);
  await expect(page.getByRole("button", { name: "Create admin" })).toBeVisible();
});
