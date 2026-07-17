import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

// support/server.js -> e2e/support -> e2e -> repo root
const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  ".."
);
const BINARY = path.join(repoRoot, "target", "debug", "pingward");

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

async function waitForServer(url, getSpawnError, timeoutMs = 30000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const spawnError = getSpawnError();
    if (spawnError) throw spawnError;
    try {
      const res = await fetch(`${url}/healthz`);
      if (res.ok) return;
    } catch {
      // server not up yet
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error(`pingward did not become ready at ${url} within ${timeoutMs}ms`);
}

export async function spawnPingward({ scanIntervalSecs } = {}) {
  const dir = mkdtempSync(path.join(tmpdir(), "pingward-e2e-"));
  const dbPath = path.join(dir, "test.sqlite3");
  const port = await findAvailablePort();
  const url = `http://127.0.0.1:${port}`;

  const proc = spawn(BINARY, [], {
    cwd: repoRoot, // sqlx migrations use relative paths
    env: {
      ...process.env,
      DATABASE_URL: `sqlite://${dbPath}?mode=rwc`,
      PINGWARD_BIND: `127.0.0.1:${port}`,
      PINGWARD_BASE_URL: url,
      RUST_LOG: "warn",
      ...(scanIntervalSecs != null
        ? { PINGWARD_SCAN_INTERVAL: String(scanIntervalSecs) }
        : {}),
    },
    stdio: "ignore",
  });

  let spawnError = null;
  proc.on("error", (e) => {
    spawnError = e;
  });

  const cleanup = async () => {
    proc.kill("SIGTERM");
    rmSync(dir, { recursive: true, force: true });
  };

  try {
    await waitForServer(url, () => spawnError);
  } catch (err) {
    await cleanup();
    throw err;
  }

  return { url, dbPath, cleanup };
}
