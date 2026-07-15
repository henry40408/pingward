# Authorization & Security Boundaries E2E Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a browser-level E2E feature that verifies pingward's authorization and security boundaries (admin-only surface, cross-user ownership, unauthenticated redirect, CSRF rejection).

**Architecture:** Reuse the established `e2e/` harness (Approach A: per-scenario fresh compiled binary + temp SQLite; playwright-bdd; `data-testid` selectors). Add additive `data-testid` hooks to two templates, a scenario-scoped `world` fixture for cross-step state, and one new `authz.feature` + `authz.steps.js`. No application (`src/*`) behavior changes.

**Tech Stack:** Playwright, playwright-bdd (Gherkin), Rust/axum/askama (templates compile into the binary at build time), SQLite.

## Global Constraints

- **Additive only:** every change is a new `data-testid`, a new fixture, or new test files. Existing `id`/`class`/`name`/`_csrf` attributes and markup are preserved; `src/*` behavior is unchanged (`git diff --name-only -- 'src/*'` MUST be empty).
- **Rebuild after template edits:** askama compiles templates into the binary. Run `cargo build` after any `templates/*.html` change or the E2E harness (which spawns the compiled binary) will serve stale HTML.
- **Selectors:** stable `data-testid`, matching the auth/monitoring batches.
- **Cross-user access returns 404, not 403:** `owned_project` returns `AppError::NotFound` (renders 404 "not found") to hide a resource's existence. 403 is only for the `/admin/*` `AdminUser` guard and the CSRF guard.
- **Step file imports:** step definitions and fixtures import `{ test, expect }` from `../support/fixtures.js` — NEVER from `@playwright/test` (that would bypass the `pingwardServer`/`serverUrl`/`api`/`world` fixtures).
- **Commits:** GPG-signed; stage files explicitly by name (never `git add -A`/`.`); trailer `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- **Branch:** implement on `test/authz-boundaries-e2e` (already created); do NOT create new branches.
- **Rust tests:** `cargo nextest run` (not `cargo test`).

---

## File Structure

- `templates/base.html` — add `data-testid` to the two admin-only nav links (Task 1).
- `templates/users.html` — add `data-testid` to the create-user form fields (Task 1).
- `e2e/support/fixtures.js` — add the `world` fixture (Task 2).
- `e2e/features/authz.feature` — the six-scenario behavioural spec (Task 3).
- `e2e/steps/authz.steps.js` — step definitions for the new steps (Task 3).

---

## Task 1: Add data-testid hooks to templates + rebuild

**Files:**
- Modify: `templates/base.html:16`
- Modify: `templates/users.html:46`, `templates/users.html:50`, `templates/users.html:58`

**Interfaces:**
- Consumes: nothing.
- Produces: five `data-testid` hooks the Task 3 steps rely on — `nav-settings`, `nav-admin` (base.html); `user-username-input`, `user-password-input`, `user-submit` (users.html).

- [ ] **Step 1: Add testids to the admin-only nav links**

In `templates/base.html`, line 16 currently reads:

```html
    <nav class="links"><a href="/">Dashboard</a>{% if is_admin %}<a href="/settings">Settings</a><a href="/admin">Admin</a>{% endif %}</nav>
```

Replace it with (adding `data-testid` to the two admin links only):

```html
    <nav class="links"><a href="/">Dashboard</a>{% if is_admin %}<a href="/settings" data-testid="nav-settings">Settings</a><a href="/admin" data-testid="nav-admin">Admin</a>{% endif %}</nav>
```

- [ ] **Step 2: Add testids to the create-user form**

In `templates/users.html`, line 46 currently reads:

```html
          <input id="username" name="username" required>
```

Replace with:

```html
          <input id="username" name="username" data-testid="user-username-input" required>
```

Line 50 currently reads:

```html
          <input id="password" name="password" type="password" required>
```

Replace with:

```html
          <input id="password" name="password" type="password" data-testid="user-password-input" required>
```

Line 58 currently reads:

```html
        <button class="btn primary" type="submit">Create user</button>
```

Replace with:

```html
        <button class="btn primary" type="submit" data-testid="user-submit">Create user</button>
```

- [ ] **Step 3: Rebuild the binary (templates compile in)**

Run: `cargo build`
Expected: `Finished` with no errors (askama recompiles the edited templates into the binary).

- [ ] **Step 4: Sanity-check the hooks landed**

Run: `rg -c 'data-testid="(nav-settings|nav-admin)"' templates/base.html && rg -c 'data-testid="user-(username-input|password-input|submit)"' templates/users.html`
Expected: `2` (base.html) then `3` (users.html).

- [ ] **Step 5: Confirm no Rust view test regressed**

Run: `cargo nextest run`
Expected: PASS (the additive attributes change no asserted markup; the suite was 187/187 before this batch).

- [ ] **Step 6: Commit**

```bash
git add templates/base.html templates/users.html
git commit -S -m "test: add data-testid hooks for authz boundary E2E

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 2: Add the `world` fixture

**Files:**
- Modify: `e2e/support/fixtures.js`

**Interfaces:**
- Consumes: nothing.
- Produces: a test-scoped `world` fixture — a fresh plain object `{}` per scenario — that Task 3 steps read/write via `world.status` (last captured HTTP status) and `world.projectUrl` (a remembered project page URL).

- [ ] **Step 1: Add the `world` fixture**

In `e2e/support/fixtures.js`, the `test` object is currently:

```javascript
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
```

Add a `world` fixture after `api` (still inside `base.extend({ ... })`):

```javascript
  api: async ({ serverUrl }, use) => {
    await use(new ApiHelper(serverUrl));
  },
  // Scenario-scoped scratch object for carrying state across steps within a
  // single scenario (e.g. a remembered project URL, the last HTTP status).
  world: async ({}, use) => {
    await use({});
  },
```

- [ ] **Step 2: Verify the file parses**

Run: `node --check e2e/support/fixtures.js`
Expected: no output, exit 0.

- [ ] **Step 3: Commit**

```bash
git add e2e/support/fixtures.js
git commit -S -m "test: add world fixture for cross-step authz state

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Task 3: `authz.feature` + `authz.steps.js`

**Files:**
- Create: `e2e/features/authz.feature`
- Create: `e2e/steps/authz.steps.js`

**Interfaces:**
- Consumes: the `data-testid` hooks from Task 1 (`nav-settings`, `nav-admin`, `user-username-input`, `user-password-input`, `user-submit`); the `world` fixture from Task 2; existing auth steps (`I am signed in as …`, `I sign out`, `I am on the login page`) and the monitoring step `I create a project named {string}`; existing auth-form testids `username-input`, `password-input`, `login-submit`, `logout-button`.
- Produces: nothing downstream.

- [ ] **Step 1: Write the feature file**

Create `e2e/features/authz.feature` with exactly:

```gherkin
Feature: Authorization and security boundaries

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario: An admin sees the Settings and Admin nav links
    Then the "Settings" nav link is visible
    And the "Admin" nav link is visible

  Scenario: A non-admin does not see the Settings or Admin nav links
    Given a non-admin user "member" with password "hunter2 correct" exists
    And I sign out
    And I am signed in as "member" with password "hunter2 correct"
    Then the "Settings" nav link is not visible
    And the "Admin" nav link is not visible

  Scenario: A non-admin is forbidden from the admin area
    Given a non-admin user "member" with password "hunter2 correct" exists
    And I sign out
    And I am signed in as "member" with password "hunter2 correct"
    When I navigate to "/admin"
    Then the response status is 403

  Scenario: A non-admin cannot read another user's project
    Given a non-admin user "member" with password "hunter2 correct" exists
    And I create a project named "Secret jobs"
    And I remember the current project
    When I revisit it as "member" with password "hunter2 correct"
    Then the response status is 404

  Scenario: A logged-out visitor is redirected to login
    Given I sign out
    When I navigate to "/"
    Then I am on the login page

  Scenario: A POST without a CSRF token is rejected
    When I POST to "/projects" without a CSRF token
    Then the response status is 403
```

- [ ] **Step 2: Write the step definitions**

Create `e2e/steps/authz.steps.js` with exactly:

```javascript
import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Given, When, Then } = createBdd(test);

// Human nav label -> its data-testid on the admin-only nav links.
const NAV_TESTID = { Settings: "nav-settings", Admin: "nav-admin" };

// Create a non-admin user through the admin-only /users form. Assumes the
// admin is signed in (the form is guarded by AdminUser). The is_admin
// checkbox is left unchecked, so the created user is a non-admin. On success
// the handler redirects back to /users, where the new username is listed.
Given(
  "a non-admin user {string} with password {string} exists",
  async ({ page, serverUrl }, username, password) => {
    await page.goto(`${serverUrl}/users`);
    await page.getByTestId("user-username-input").fill(username);
    await page.getByTestId("user-password-input").fill(password);
    await page.getByTestId("user-submit").click();
    await expect(page).toHaveURL(`${serverUrl}/users`);
    await expect(page.getByText(username, { exact: true }).first()).toBeVisible();
  }
);

Then("the {string} nav link is visible", async ({ page }, label) => {
  await expect(page.getByTestId(NAV_TESTID[label])).toBeVisible();
});

Then("the {string} nav link is not visible", async ({ page }, label) => {
  await expect(page.getByTestId(NAV_TESTID[label])).toHaveCount(0);
});

// Navigate to a path and record the final response status on `world`.
When("I navigate to {string}", async ({ page, serverUrl, world }, path) => {
  const res = await page.goto(`${serverUrl}${path}`);
  world.status = res.status();
});

Then("the response status is {int}", async ({ world }, status) => {
  expect(world.status).toBe(status);
});

// Issue an authenticated POST with no _csrf field. page.request shares the
// browser context's cookies (the session cookie rides along), so this hits
// the CSRF guard as a real logged-in request that simply omits the token.
When(
  "I POST to {string} without a CSRF token",
  async ({ page, serverUrl, world }, path) => {
    const res = await page.request.post(`${serverUrl}${path}`, { form: {} });
    world.status = res.status();
  }
);

// Remember the current page URL (a project page) for a later cross-user visit.
// Wait for the project URL first: the monitoring "I create a project named"
// step clicks submit without awaiting the redirect, so page.url() could still
// read /projects/new if we captured it immediately.
When("I remember the current project", async ({ page, world }) => {
  await expect(page).toHaveURL(/\/projects\/\d+$/);
  world.projectUrl = page.url();
});

// Sign out, sign in as another user, revisit the remembered project URL, and
// record the response status (expected 404: the project exists but is owned by
// someone else, and owned_project hides existence).
When(
  "I revisit it as {string} with password {string}",
  async ({ page, serverUrl, world }, username, password) => {
    await page.getByTestId("logout-button").click();
    await page.goto(`${serverUrl}/login`);
    await page.getByTestId("username-input").fill(username);
    await page.getByTestId("password-input").fill(password);
    await page.getByTestId("login-submit").click();
    await expect(page).toHaveURL(`${serverUrl}/`);
    const res = await page.goto(world.projectUrl);
    world.status = res.status();
  }
);
```

- [ ] **Step 3: Generate the BDD specs**

Run: `cd e2e && npx bddgen`
Expected: completes without error; generates `.features-gen/features/authz.feature.spec.js`.

- [ ] **Step 4: Run the new feature and verify all six scenarios pass**

Run: `cd e2e && npx bddgen && npx playwright test authz`
Expected: `6 passed`.

- [ ] **Step 5: Run the full E2E suite (no regression in auth/monitoring)**

Run: `cd e2e && npx bddgen && npx playwright test`
Expected: `21 passed` (5 auth + 10 monitoring + 6 authz).

- [ ] **Step 6: Commit**

```bash
git add e2e/features/authz.feature e2e/steps/authz.steps.js
git commit -S -m "test: add authorization & security boundary E2E feature and steps

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Notes for the executor

- **playwright-bdd keyword independence:** Gherkin `Given`/`When`/`Then`/`And` keywords do NOT participate in step matching. The feature uses `And I create a project named "Secret jobs"` which binds to the monitoring `When("I create a project named {string}")` — this is expected and correct.
- **Session switching:** scenarios that act as `member` first create the user (admin session), then `sign out` and sign in as `member`. Do not reorder — the create-user form needs the admin session.
- **Scenario 5 asserts URL, not status:** a logged-out `GET /` redirects and resolves to a 200 `/login` page, so it is verified with `I am on the login page` (URL check), not a status code.
