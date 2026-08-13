import { defineConfig, devices } from "@playwright/test";
import { defineBddConfig } from "playwright-bdd";

const testDir = defineBddConfig({
  features: "features/**/*.feature",
  // playwright-bdd auto-detects the custom `test` instance by scanning the
  // "steps" files for one that calls base.extend(); it must include the
  // fixtures file (not just steps/**/*.js) or bddgen can't find it.
  steps: ["steps/**/*.js", "support/fixtures.js"],
});

export default defineConfig({
  testDir,
  globalSetup: "./global-setup.js",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: process.env.CI ? [["html", { open: "never" }], ["list"]] : "list",
  use: {
    trace: "on-first-retry",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
      // `no_js.feature` is the same app with scripting switched off, which is
      // a browser capability rather than anything the page can be asked to do
      // — so it needs its own context, and must not also run here.
      testIgnore: /no_js\.feature\.spec\.js$/,
    },
    {
      // Everything the UI promises that `app.js` is not allowed to be the only
      // provider of. The rest of the suite runs with script on and is blind to
      // this by construction.
      name: "no-js",
      use: { ...devices["Desktop Chrome"], javaScriptEnabled: false },
      testMatch: /no_js\.feature\.spec\.js$/,
    },
  ],
});
