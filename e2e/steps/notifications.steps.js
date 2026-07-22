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
  const banner = page.getByTestId("project-channels-empty");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("nobody is notified");
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

// After saving notify channels the check page shows a one-shot success flash
// (backed by a flash cookie that is cleared on this render).
Then("a {string} confirmation is shown", async ({ page }, msg) => {
  await expect(page.getByTestId("check-flash")).toHaveText(msg);
});

// The flash is one-shot: reloading the check page must NOT show it again.
Then("the confirmation is gone after reloading", async ({ page }) => {
  await page.reload();
  await expect(page.getByTestId("check-flash")).toHaveCount(0);
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

// From the project page, click into a check's row (a `role="link"` div, not a
// real anchor) to reach its check page.
When("I visit the check page for {string}", async ({ page }, name) => {
  await page.locator(".check", { hasText: name }).click();
  await expect(page).toHaveURL(/\/checks\/\d+$/);
});

When("I visit the dashboard", async ({ page, serverUrl }) => {
  await page.goto(`${serverUrl}/`);
});

// Each notify-channel row carries `data-testid="channel-state-N"` wrapping
// both the ".on" and ".off" spans (always both in the DOM, CSS shows exactly
// one keyed off the checkbox's live state). Assert visibility of each span
// directly rather than `toHaveText`, which reads `textContent` and would see
// "ONOFF" regardless of which one is actually displayed.
Then(
  "the channel {string} shows as ON on the check page",
  async ({ page }, name) => {
    const onoff = page
      .locator("label.chk", { hasText: name })
      .locator('[data-testid^="channel-state-"]');
    await expect(onoff.locator(".on")).toBeVisible();
    await expect(onoff.locator(".off")).toBeHidden();
  }
);

Then(
  "the channel {string} shows as OFF on the check page",
  async ({ page }, name) => {
    const onoff = page
      .locator("label.chk", { hasText: name })
      .locator('[data-testid^="channel-state-"]');
    await expect(onoff.locator(".off")).toBeVisible();
    await expect(onoff.locator(".on")).toBeHidden();
  }
);

// The "no channel" chip (data-testid="check-no-channel") only renders on a
// dashboard row for a check with zero bound channels — asserted both
// directions (present / absent) across the two scenario checks.
Then(
  "the dashboard shows a {string} chip for the check {string}",
  async ({ page }, chip, name) => {
    const row = page.getByTestId("dashboard-check-row").filter({ hasText: name });
    await expect(
      row.getByTestId("check-no-channel"),
      `the check "${name}" has no bound channel, so its row must carry the "${chip}" chip`
    ).toHaveText(chip);
  }
);

Then(
  "the dashboard shows no {string} chip for the check {string}",
  async ({ page }, chip, name) => {
    const row = page.getByTestId("dashboard-check-row").filter({ hasText: name });
    // Non-vacuity guard: both assertions below are trivially satisfied when
    // `row` matches nothing (a chip inside a missing row has count 0, and
    // Playwright treats `not.toContainText` against zero elements as met), so
    // a scenario that never created the check would pass without this.
    await expect(
      row,
      `no dashboard row for the check "${name}" — the absence assertions below would pass vacuously`
    ).toHaveCount(1);
    await expect(
      row.getByTestId("check-no-channel"),
      `the check "${name}" is bound to a channel, so its row must not carry the chip`
    ).toHaveCount(0);
    // Also assert the wording itself is absent, so re-rendering the same
    // warning under a different testid would still fail this scenario.
    await expect(
      row,
      `the check "${name}" is bound to a channel, yet its row still reads "${chip}"`
    ).not.toContainText(chip);
  }
);

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

// Recent-notifications event cell renders as .pill.{class}, mirroring the
// ping-kind pills. Map the Gherkin event label to its pill class. Scope to the
// notifications section so a ping's .pill.fail can't satisfy a "down" event.
const EVENT_PILL_CLASS = { down: "fail", up: "ok", reminder: "start" };

Then(
  "the recent notifications table shows a {string} event",
  async ({ page }, event) => {
    const cls = EVENT_PILL_CLASS[event];
    await expect(
      page.locator(`#notifs-section .pill.${cls}`).first()
    ).toBeVisible();
  }
);
