import { execSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export default function globalSetup() {
  const binary = path.join(repoRoot, "target", "debug", "pingward");
  if (existsSync(binary)) {
    // NOTE: pingward embeds templates/assets at compile time (askama / include_*),
    // so rebuild manually (`cargo build`) after changing app code or templates.
    return;
  }
  execSync("cargo build", { cwd: repoRoot, stdio: "inherit" });
}
