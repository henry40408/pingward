import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Then } = createBdd(test);

// `DOMParser` with the image/svg+xml type is a real, spec-compliant XML parser
// — it is what the browser itself uses for the asset — and it reports a
// failure by returning a document whose root is a <parsererror> rather than by
// throwing. Fetching from inside the page keeps the request same-origin and
// exercises the served response, headers and all.
Then("{string} is well-formed XML", async ({ page }, assetPath) => {
  const result = await page.evaluate(async (path) => {
    const res = await fetch(path);
    if (!res.ok) return { status: res.status, error: null };
    const doc = new DOMParser().parseFromString(await res.text(), "image/svg+xml");
    const failure = doc.querySelector("parsererror");
    return { status: res.status, error: failure && failure.textContent.trim() };
  }, assetPath);

  expect(result.status, `${assetPath} returned HTTP ${result.status}`).toBe(200);
  expect(
    result.error,
    `${assetPath} is not well-formed XML, so a strict parser renders no icon at all:\n${result.error}`
  ).toBeNull();
});
