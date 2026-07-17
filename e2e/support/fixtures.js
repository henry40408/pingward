import { test as base } from "playwright-bdd";
import { expect } from "@playwright/test";
import { spawnPingward } from "./server.js";
import { ApiHelper } from "./api.js";
import { startMockServer } from "./mock-http.js";

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
  // A mock HTTP endpoint that records received requests, used to assert webhook
  // test-send and down/up delivery. Test-scoped: fresh per scenario, lazily
  // instantiated only when a step requests it.
  mockWebhook: async ({}, use) => {
    const mock = await startMockServer();
    try {
      await use(mock);
    } finally {
      await mock.cleanup();
    }
  },
});

export { expect };
