import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Then } = createBdd(test);

// The version string is `git describe` output, which has no single shape: a
// release tag, a tag plus distance, or a bare short SHA before the first tag
// exists (and from CI's shallow, tag-less checkout). So this asserts the
// footer rendered *something* rather than matching a semver pattern that
// would fail on a perfectly good build.
Then("the footer shows the build version", async ({ page }) => {
  const version = page.getByTestId("app-version");
  await expect(version).toBeVisible();
  await expect(version).toHaveText(/^pingward \S+$/);
});

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
