import { test as base } from "playwright-bdd";
import { expect } from "@playwright/test";
import { spawnPingward } from "./server.js";
import { ApiHelper } from "./api.js";

export const test = base.extend({
  // One fresh server + temp DB per scenario (test-scoped).
  pingwardServer: async ({}, use) => {
    const server = await spawnPingward();
    try {
      await use(server);
    } finally {
      await server.cleanup();
    }
  },
  serverUrl: async ({ pingwardServer }, use) => {
    await use(pingwardServer.url);
  },
  api: async ({ serverUrl }, use) => {
    await use(new ApiHelper(serverUrl));
  },
  // Scenario-scoped scratch object for carrying state across steps within a
  // single scenario (e.g. a remembered project URL, the last HTTP status).
  world: async ({}, use) => {
    await use({});
  },
});

export { expect };
