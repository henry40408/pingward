// Render assets/apple-touch-icon.png from assets/favicon.svg.
//
// iOS needs a raster home-screen icon, but a second hand-drawn artwork would
// drift from the SVG. Rather than pull a Rust SVG rasteriser into the build,
// this reuses the Chromium that Playwright already installs for the E2E suite.
// Run from e2e/:  npm run icons
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
  ".."
);
const SRC = path.join(repoRoot, "assets", "favicon.svg");
const OUT = path.join(repoRoot, "assets", "apple-touch-icon.png");
const SIZE = 180; // the size iOS asks for; it downscales for smaller slots.

const svg = readFileSync(SRC, "utf8");

// Square off the two `data-frame` rects: iOS masks the icon with its own
// superellipse, and a source radius under that mask reads as a double-rounded
// edge. Chromium honours `rx` as a CSS property, so this needs no rewriting of
// the SVG source (and leaves the inner quadrants' radius alone).
const STYLE = `html,body{margin:0}svg{display:block;width:${SIZE}px;height:${SIZE}px}[data-frame]{rx:0}`;

const browser = await chromium.launch();
try {
  const page = await browser.newPage({
    viewport: { width: SIZE, height: SIZE },
    deviceScaleFactor: 1,
  });
  await page.setContent(`<style>${STYLE}</style>${svg}`, { waitUntil: "load" });
  writeFileSync(OUT, await page.screenshot({ clip: { x: 0, y: 0, width: SIZE, height: SIZE } }));
} finally {
  await browser.close();
}
console.log(`wrote ${path.relative(repoRoot, OUT)} (${SIZE}x${SIZE})`);
