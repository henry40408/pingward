// Re-runnable README screenshot pipeline.
//
//   wipe DB -> boot #1 (migrations) -> POST /setup -> SIGTERM
//   -> sqlite3 seed (backdated demo history) -> boot #2 -> log in
//   -> capture -> SIGTERM
//
// Run from e2e/:  npm run screenshots
import { spawn } from "node:child_process";
import { mkdirSync, rmSync } from "node:fs";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import { ADMIN_PASSWORD, ADMIN_USERNAME, generateSeedSql } from "./seed.mjs";

const E2E_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REPO_ROOT = path.resolve(E2E_DIR, "..");
const BINARY = process.env.PINGWARD_BIN ?? path.join(REPO_ROOT, "target", "debug", "pingward");
const DB = path.join(E2E_DIR, ".tmp", "screenshots.sqlite3");
const OUT = path.join(REPO_ROOT, "docs", "screenshots");

// The bind port is throwaway, but the *rendered* ping URLs come from
// PINGWARD_BASE_URL — so point that at a plausible public hostname instead of
// baking a random loopback port into the check-page screenshot.
const PUBLIC_BASE_URL = "https://pingward.example.com";

function findAvailablePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.unref();
    srv.on("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const { port } = srv.address();
      srv.close(() => resolve(port));
    });
  });
}

async function startPingward(port) {
  const proc = spawn(BINARY, [], {
    cwd: REPO_ROOT,
    env: {
      ...process.env,
      DATABASE_URL: `sqlite://${DB}?mode=rwc`,
      PINGWARD_BIND: `127.0.0.1:${port}`,
      PINGWARD_BASE_URL: PUBLIC_BASE_URL,
      RUST_LOG: "warn",
    },
    stdio: ["ignore", "ignore", "inherit"],
  });
  proc.exited = new Promise((r) => proc.once("exit", r));
  const base = `http://127.0.0.1:${port}`;
  const deadline = Date.now() + 30_000;
  while (Date.now() < deadline) {
    try {
      if ((await fetch(`${base}/healthz`)).ok) return proc;
    } catch {
      // not listening yet
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  proc.kill("SIGKILL");
  throw new Error(`pingward did not become ready at ${base}`);
}

// SIGTERM is a graceful shutdown: it closes the pool, which checkpoints the WAL
// and removes the sidecars — so the stopped file is safe for the sqlite3 CLI.
async function stopPingward(proc) {
  proc.kill("SIGTERM");
  const hardKill = setTimeout(() => proc.kill("SIGKILL"), 10_000);
  await proc.exited;
  clearTimeout(hardKill);
}

function runSqlite(sql) {
  return new Promise((resolve, reject) => {
    const p = spawn("sqlite3", [DB], { stdio: ["pipe", "inherit", "inherit"] });
    p.once("error", reject);
    p.once("exit", (code) =>
      code === 0 ? resolve() : reject(new Error(`sqlite3 exited ${code}`))
    );
    p.stdin.end(sql);
  });
}

const DESKTOP = { viewport: { width: 1280, height: 900 }, deviceScaleFactor: 2 };
const MOBILE = {
  viewport: { width: 390, height: 844 },
  deviceScaleFactor: 3,
  isMobile: true,
  hasTouch: true,
};

const PAD = 16;

// Document-relative box of `locator`. Every capture scrolls back to the top
// first, so the viewport-relative box Playwright reports *is* the document
// box — which is the coordinate space `screenshot({ fullPage: true, clip })`
// expects.
async function boxOf(page, locator) {
  await page.evaluate(() => window.scrollTo(0, 0));
  const box = await locator.boundingBox();
  if (!box) throw new Error("element to clip to is not visible");
  return box;
}

// Clip from the top of the page down to just past `locator`, so a shot ends
// on a card boundary instead of slicing through a table. `pad` is 0 when the
// cut lands on a list divider, where any padding would leak a sliver of the
// next row.
const downTo = (locate, pad = PAD) => async (page, width) => {
  const box = await boxOf(page, locate(page));
  return { x: 0, y: 0, width, height: Math.ceil(box.y + box.height + pad) };
};

// Clip to the band spanned by `from`..`to`, for a shot of the middle of a
// long page.
const band = (from, to) => async (page, width) => {
  const head = await boxOf(page, from(page));
  const tail = await boxOf(page, to(page));
  const y = Math.max(0, Math.floor(head.y - PAD));
  return { x: 0, y, width, height: Math.ceil(tail.y + tail.height + PAD - y) };
};

// Named page regions, so the shot list reads as intent rather than selectors.
const schedulerCard = (page) =>
  page.locator(".card").filter({ has: page.getByTestId("sched-scan") });
const pingsCard = (page) => page.locator("#pings-card");
const lastCard = (page) => page.locator(".card").last();
const channelsCard = (page) =>
  page.locator(".card").filter({ has: page.getByText("Notify channels") });
const nthCheckRow = (n) => (page) => page.getByTestId("dashboard-check-row").nth(n);

async function openDownCheck(page) {
  await page.getByText("home-nas-snapshot", { exact: true }).click();
  await page.getByTestId("check-status").waitFor();
  await page.getByTestId("ping-row").first().waitFor();
}

// `goto` is a path; `settle` leaves the page in the state worth photographing;
// `clip` (optional) narrows the frame. `fullPage` alone is only right for the
// pages that end on their own — the check and admin pages are long enough that
// a whole-page capture reads as a strip in a README.
const SHOTS = [
  {
    file: "dashboard-dark.png",
    scheme: "dark",
    ...DESKTOP,
    goto: "/",
    fullPage: true,
    settle: (page) => page.getByTestId("dashboard-check-row").nth(9).waitFor(),
  },
  {
    file: "check-dark.png",
    scheme: "dark",
    ...DESKTOP,
    goto: "/",
    settle: openDownCheck,
    clip: downTo(channelsCard),
  },
  {
    file: "check-history-dark.png",
    scheme: "dark",
    ...DESKTOP,
    goto: "/",
    settle: async (page) => {
      await openDownCheck(page);
      // Expand the failed run so its captured output is part of the shot —
      // that body is what a `curl --data-binary @- …/fail` sends.
      await page.locator("tr.toggle").first().click();
      await page.locator("tr.exp .out").first().waitFor();
    },
    clip: band(pingsCard, lastCard),
  },
  {
    file: "project-dark.png",
    scheme: "dark",
    ...DESKTOP,
    goto: "/",
    fullPage: true,
    settle: async (page) => {
      await page.getByRole("link", { name: "Manage →" }).first().click();
      await page.getByTestId("new-check-link").waitFor();
    },
  },
  {
    file: "admin-dark.png",
    scheme: "dark",
    ...DESKTOP,
    goto: "/admin",
    settle: (page) => page.getByTestId("admin-scale").waitFor(),
    clip: downTo(schedulerCard),
  },
  {
    file: "dashboard-light.png",
    scheme: "light",
    ...DESKTOP,
    goto: "/",
    fullPage: true,
    settle: (page) => page.getByTestId("dashboard-check-row").nth(9).waitFor(),
  },
  {
    file: "dashboard-mobile.png",
    scheme: "dark",
    ...MOBILE,
    goto: "/",
    settle: (page) => page.getByTestId("dashboard-check-row").nth(1).waitFor(),
    clip: downTo(nthCheckRow(1), 0),
  },
  {
    file: "check-mobile.png",
    scheme: "dark",
    ...MOBILE,
    goto: "/",
    settle: openDownCheck,
    clip: downTo(channelsCard),
  },
];

// Freeze anything that would make two runs of the same page differ.
const FREEZE_CSS =
  "*{animation:none !important;transition:none !important;caret-color:transparent !important}";

async function main() {
  mkdirSync(path.dirname(DB), { recursive: true });
  mkdirSync(OUT, { recursive: true });
  for (const suffix of ["", "-wal", "-shm"]) rmSync(`${DB}${suffix}`, { force: true });

  const port = await findAvailablePort();
  const base = `http://127.0.0.1:${port}`;

  // Phase 1 — migrate, then create the first admin through the product's own
  // one-time setup form so the password hash is a real argon2 one.
  let server = await startPingward(port);
  try {
    const res = await fetch(`${base}/setup`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ username: ADMIN_USERNAME, password: ADMIN_PASSWORD }),
    });
    if (!res.ok) throw new Error(`POST /setup failed: HTTP ${res.status}`);
  } finally {
    await stopPingward(server);
  }

  // Phase 2 — backdated demo data, written against the stopped database.
  await runSqlite(generateSeedSql(Date.now()));

  // Phase 3 — boot on the seeded database and photograph it.
  server = await startPingward(port);
  let browser;
  try {
    browser = await chromium.launch();
    const auth = await browser.newContext({ baseURL: base });
    const loginPage = await auth.newPage();
    await loginPage.goto("/login");
    await loginPage.getByTestId("username-input").fill(ADMIN_USERNAME);
    await loginPage.getByTestId("password-input").fill(ADMIN_PASSWORD);
    await loginPage.getByTestId("login-submit").click();
    await loginPage.getByTestId("nav-admin").waitFor();
    const storageState = await auth.storageState();
    await auth.close();

    for (const shot of SHOTS) {
      const ctx = await browser.newContext({
        baseURL: base,
        storageState,
        colorScheme: shot.scheme,
        viewport: shot.viewport,
        deviceScaleFactor: shot.deviceScaleFactor,
        isMobile: shot.isMobile ?? false,
        hasTouch: shot.hasTouch ?? false,
        timezoneId: "UTC",
        locale: "en-US",
        reducedMotion: "reduce",
      });
      const page = await ctx.newPage();
      await page.goto(shot.goto);
      await shot.settle(page);
      await page.addStyleTag({ content: FREEZE_CSS });
      await page.evaluate(() => document.fonts.ready);
      const clip = shot.clip ? await shot.clip(page, shot.viewport.width) : undefined;
      await page.screenshot({
        path: path.join(OUT, shot.file),
        // A clip is always document-relative, which only holds for a full-page
        // capture — see `boxOf`.
        fullPage: shot.fullPage ?? clip != null,
        clip,
      });
      console.log(`captured ${shot.file}`);
      await ctx.close();
    }
  } finally {
    if (browser) await browser.close();
    await stopPingward(server);
  }
  console.log(`done — PNGs in ${path.relative(REPO_ROOT, OUT)}`);
}

main().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
