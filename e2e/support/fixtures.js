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
});

export { expect };
