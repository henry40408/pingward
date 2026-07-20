import { test as base } from "playwright-bdd";
import { expect } from "@playwright/test";
import { spawnPingward } from "./server.js";
import { ApiHelper } from "./api.js";
import { startMockServer } from "./mock-http.js";

export const test = base.extend({
  // One fresh server + temp DB per scenario (test-scoped).
  pingwardServer: async ({ $tags }, use) => {
    // Scenarios tagged @fast-scan spawn pingward with a 1s scan interval so the
    // background scan loop transitions overdue/overrun checks to down within a
    // couple of seconds (time_states.feature). Untagged scenarios keep the
    // default (~30s) interval — a fast scan for them would add needless churn.
    const opts = {};
    if ($tags.includes("@fast-scan")) opts.scanIntervalSecs = 1;
    // Scenarios tagged @smtp-env spawn pingward with instance SMTP configured,
    // so the /admin Environment card's SMTP group has something to show as
    // "configured" (admin.feature).
    if ($tags.includes("@smtp-env")) {
      opts.extraEnv = {
        PINGWARD_SMTP_HOST: "smtp.e2e.test",
        PINGWARD_SMTP_FROM: "alerts@e2e.test",
        PINGWARD_SMTP_PASSWORD: "e2e-secret-password",
      };
    }
    // Scenarios tagged @trusted-proxy trust the loopback address the harness
    // connects from, so auth::client_ip honours their X-Forwarded-For instead
    // of recording the peer (account.feature).
    if ($tags.includes("@trusted-proxy")) {
      opts.extraEnv = { ...opts.extraEnv, PINGWARD_TRUSTED_PROXIES: "127.0.0.1" };
    }
    const server = await spawnPingward(opts);
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
