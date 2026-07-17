import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Given, When, Then } = createBdd(test);

// The channel form and project "Channels" section carry no data-testid, so
// everything here is driven by input ids / element classes / text, matching
// admin.feature.
//
// Fill the channel form for `kind` with canned config and submit. The kind
// <select> toggles which .cfg block is visible via inline JS, and fill() needs
// a visible target, so selectOption(kind) BEFORE filling that kind's inputs.
// A valid create redirects back to the project page.
async function createChannel(page, projectUrl, kind, name, overrides = {}) {
  await page.goto(`${projectUrl}/channels/new`);
  await page.locator("#name").fill(name);
  await page.locator("#kind").selectOption(kind);
  switch (kind) {
    case "webhook":
      await page
        .locator("#webhook_url")
        .fill(overrides.url ?? "http://127.0.0.1:1/hook");
      break;
    case "slack":
      await page
        .locator("#slack_url")
        .fill("https://hooks.slack.com/services/T0/B0/xxx");
      break;
    case "telegram":
      await page.locator("#telegram_token").fill("123:ABC");
      await page.locator("#telegram_chat_id").fill("999");
      break;
    case "ntfy":
      await page.locator("#ntfy_topic").fill("my-topic");
      break;
    case "pushover":
      await page.locator("#pushover_token").fill("apptok");
      await page.locator("#pushover_user").fill("userkey");
      break;
    default:
      throw new Error(`unsupported channel kind in test: ${kind}`);
  }
  await Promise.all([
    page.waitForURL(/\/projects\/\d+$/),
    page.getByRole("button", { name: "Create channel" }).click(),
  ]);
}

When(
  "I create a {word} channel named {string}",
  async ({ page, world }, kind, name) => {
    await createChannel(page, world.projectUrl, kind, name);
  }
);

Given(
  "a webhook channel named {string} targeting the mock server",
  async ({ page, world, mockWebhook }, name) => {
    await createChannel(page, world.projectUrl, "webhook", name, {
      url: `${mockWebhook.url}/hook`,
    });
  }
);

Then(
  "the project lists a channel named {string} of kind {string}",
  async ({ page }, name, kind) => {
    const row = page.locator(".chk", { hasText: name });
    await expect(row).toBeVisible();
    await expect(row.locator(".kind")).toHaveText(kind);
  }
);

// Submit the webhook branch with an empty URL to hit server-side validation.
When("I submit a webhook channel with a blank URL", async ({ page, world }) => {
  await page.goto(`${world.projectUrl}/channels/new`);
  await page.locator("#name").fill("bad hook");
  await page.locator("#kind").selectOption("webhook");
  await page.getByRole("button", { name: "Create channel" }).click();
});

Then("the channel form shows an error {string}", async ({ page }, msg) => {
  await expect(page.locator(".flash.err")).toHaveText(msg);
});

When("I open the new channel form", async ({ page, world }) => {
  await page.goto(`${world.projectUrl}/channels/new`);
});

Then("the {string} channel kind is not offered", async ({ page }, kind) => {
  await expect(page.locator(`#kind option[value="${kind}"]`)).toHaveCount(0);
});

// The channel row's delete form redirects back to the same /projects/{id} URL,
// so wait on the navigation event (not the URL, which is unchanged) to avoid a
// stale-DOM read.
When("I delete the channel named {string}", async ({ page }, name) => {
  const row = page.locator(".chk", { hasText: name });
  await Promise.all([
    page.waitForNavigation(),
    row.getByRole("button", { name: "delete" }).click(),
  ]);
});

Then("the project shows no channels", async ({ page }) => {
  await expect(page.getByText("No channels yet.")).toBeVisible();
});

// On the check page, the notify-channels form lists each project channel as a
// checkbox inside a <label class="chk">. Check the box and save. Save redirects
// to the same /checks/{id} URL, so wait on navigation before asserting.
When("I bind the channel {string} to the check", async ({ page }, name) => {
  const box = page
    .locator("label.chk", { hasText: name })
    .locator('input[name="channel_ids"]');
  await box.check();
  await Promise.all([
    page.waitForNavigation(),
    page.getByRole("button", { name: "Save channels" }).click(),
  ]);
});

Then("the channel {string} is bound to the check", async ({ page }, name) => {
  const box = page
    .locator("label.chk", { hasText: name })
    .locator('input[name="channel_ids"]');
  await expect(box).toBeChecked();
});

// After saving notify channels the check page redirects with ?saved=channels
// and shows a success flash.
Then("a {string} confirmation is shown", async ({ page }, msg) => {
  await expect(page.getByTestId("check-flash")).toHaveText(msg);
});

// The "Send test" form re-renders the project page (200, no redirect) with a
// .flash banner. Click and let the following assertion auto-wait for it.
When(
  "I send a test notification to the channel {string}",
  async ({ page }, name) => {
    const row = page.locator(".chk", { hasText: name });
    await row.getByRole("button", { name: "Send test" }).click();
  }
);

Then("a channel success banner is shown", async ({ page }) => {
  await expect(page.locator(".flash.ok")).toBeVisible();
});

Then("a channel error banner is shown", async ({ page }) => {
  await expect(page.locator(".flash.err")).toBeVisible();
});

Then(
  "the mock server receives a {string} notification",
  async ({ mockWebhook }, event) => {
    await mockWebhook.waitForRequest((r) => {
      try {
        return JSON.parse(r.body).event === event;
      } catch {
        return false;
      }
    });
  }
);

// When the check's project has no channels, the Notify channels card shows an
// empty state (with a link to create one) instead of the bind form.
Then("the check's notify channels show an empty state", async ({ page }) => {
  await expect(page.getByTestId("check-channels-empty")).toBeVisible();
});

// Delivery records the notification row AFTER the webhook POST returns, so poll
// by reloading until a "sent" row for the channel appears.
Then(
  "the check's recent notifications show a delivery to {string}",
  async ({ page }, channelName) => {
    await expect(async () => {
      await page.reload();
      await expect(
        page.locator("tr", { hasText: channelName }).first()
      ).toContainText("sent");
    }).toPass({ timeout: 5000 });
  }
);
