# Core Monitoring E2E Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a browser-level E2E suite covering pingward's monitoring core — an owned user managing the Project → Check → Ping → status lifecycle.

**Architecture:** Reuse the existing `e2e/` Playwright + playwright-bdd harness (Approach A: per-scenario fresh binary + temp SQLite). Add additive `data-testid` hooks to templates, two small support-code additions (base-url env, ping helper), a new `monitoring.feature`, and its step definitions. Statuses are driven by real pings against the exact URL the UI renders.

**Tech Stack:** Playwright, playwright-bdd 9.2, Node 22, Rust/axum (app under test, rebuilt after template edits), askama SSR templates (compile-time embedded — the binary MUST be rebuilt after any template change).

## Global Constraints

- **Selectors:** every element a step touches is located by `data-testid`. Existing `id` attributes and markup are preserved (kept for label association) — additions only.
- **Isolation:** never share state between scenarios. The `pingwardServer` fixture is function-scoped; each scenario gets a fresh binary + fresh temp SQLite.
- **playwright-bdd import rule:** step/fixture files import `test` from the local `../support/fixtures.js` (which re-exports the playwright-bdd `test`), NEVER directly from `@playwright/test`. Importing `test` from `@playwright/test` breaks BDD fixture wiring.
- **Determinism:** assert only stable statuses (`new`/`up`/`down`/`paused`). Never assert `late`/overrun (they need the scheduler/time). Scenarios use `period_secs = 60`.
- **askama rebuild:** after editing any `templates/*.html`, rebuild the binary (`cargo build`) before running the E2E suite, or the running binary serves stale markup.
- **Reuse auth steps:** the Background reuses the auth batch's existing steps `Given an admin {string} with password {string} exists` and `Given I am signed in as {string} with password {string}` (in `e2e/steps/auth.steps.js`). Do not redefine them.
- **Ping URL:** the check page renders `{PINGWARD_BASE_URL}/ping/{uuid}`. The harness sets `PINGWARD_BASE_URL` to the ephemeral test server URL so the rendered URL is pingable.
- **Confirm dialogs:** delete controls submit through `confirm(...)`. Steps must register `page.on("dialog", (d) => d.accept())` before submitting.
- Follow the repo CLAUDE.md: GPG-signed commits, stage files explicitly by name (never `git add -A`/`.`), run the Rust suite with `cargo nextest run`.

## File Structure

- `templates/*.html` — add `data-testid` hooks (Task 1).
- `e2e/support/server.js` — add `PINGWARD_BASE_URL` env (Task 2).
- `e2e/support/api.js` — add `ApiHelper.ping()` (Task 2).
- `e2e/features/monitoring.feature` — new feature file (Task 3).
- `e2e/steps/monitoring.steps.js` — new step definitions (Task 3).

---

### Task 1: Add data-testid hooks to templates and rebuild

**Files:**
- Modify: `templates/project_form.html`
- Modify: `templates/check_form.html`
- Modify: `templates/check.html`
- Modify: `templates/project.html`
- Modify: `templates/dashboard.html`

**Interfaces:**
- Produces: the `data-testid` values later tasks locate — `project-name-input`, `project-submit`, `check-name-input`, `check-period-input`, `check-submit`, `check-status`, `ping-url`, `ack-button`, `pause-button`, `resume-button`, `regenerate-button`, `delete-check-button`, `new-check-link`, `delete-project-button`, `checks-empty`, `dashboard-empty`.

This task is pure template editing plus a rebuild. There is no automated unit test for template markup; the deliverable is verified by a grep assertion and a successful `cargo build` (askama compiles templates at build time, so a malformed template fails the build).

- [ ] **Step 1: Edit `templates/project_form.html`**

Add `data-testid="project-name-input"` to the name `<input>` and `data-testid="project-submit"` to the submit `<button>`. Keep existing `id="name"` and classes.

```html
<input id="name" name="name" value="{{ name }}" data-testid="project-name-input" required>
```
```html
<button class="btn primary" type="submit" data-testid="project-submit">Save changes</button>
```

- [ ] **Step 2: Edit `templates/check_form.html`**

Add `data-testid="check-name-input"` to the name input, `data-testid="check-period-input"` to the `period_secs` input, and `data-testid="check-submit"` to the submit button. Keep existing `id`s.

```html
<input id="name" name="name" value="{{ name }}" data-testid="check-name-input" required>
```
```html
<input id="period_secs" name="period_secs" value="{{ period_secs }}" data-testid="check-period-input">
```
```html
<button class="btn primary" type="submit" data-testid="check-submit">Save changes</button>
```

- [ ] **Step 3: Edit `templates/check.html`**

Add `data-testid="check-status"` to the status badge (line ~12: `<span class="badge {{ status }}">{{ status }}</span>`), `data-testid="ping-url"` to the `<code>` holding the URL (line ~34), and testids to the action buttons. Keep all existing attributes.

```html
<span class="badge {{ status }}" data-testid="check-status">{{ status }}</span>
```
```html
<div class="urlrow"><code data-testid="ping-url">{{ ping_url }}</code><button class="copy" type="button" data-copy="{{ ping_url }}">Copy</button></div>
```

In the `<div class="actions">` block, add:
- `data-testid="resume-button"` to the Resume button (inside the `status == "paused"` branch)
- `data-testid="pause-button"` to the Pause button (else branch)
- `data-testid="ack-button"` to the Acknowledge button (inside the `status == "down" && !check.acknowledged` branch)
- `data-testid="regenerate-button"` to the Regenerate URL button
- `data-testid="delete-check-button"` to the Delete button

```html
<form class="inline" method="post" action="{{ base }}/checks/{{ check.id }}/resume"><input type="hidden" name="_csrf" value="{{ csrf }}"><button class="btn" data-testid="resume-button">Resume</button></form>
```
```html
<form class="inline" method="post" action="{{ base }}/checks/{{ check.id }}/pause"><input type="hidden" name="_csrf" value="{{ csrf }}"><button class="btn" data-testid="pause-button">Pause</button></form>
```
```html
<form class="inline" method="post" action="{{ base }}/checks/{{ check.id }}/ack"><input type="hidden" name="_csrf" value="{{ csrf }}"><button class="btn primary" data-testid="ack-button">Acknowledge</button></form>
```
```html
<form class="inline" method="post" action="{{ base }}/checks/{{ check.id }}/regenerate"><input type="hidden" name="_csrf" value="{{ csrf }}"><button class="btn" data-testid="regenerate-button">Regenerate URL</button></form>
```
```html
<form class="inline" method="post" action="{{ base }}/checks/{{ check.id }}/delete"
      onsubmit="return confirm('Delete this check?')"><input type="hidden" name="_csrf" value="{{ csrf }}"><button class="btn danger" data-testid="delete-check-button">Delete</button></form>
```

- [ ] **Step 4: Edit `templates/project.html`**

Add `data-testid="new-check-link"` to the "New check" `<a>` (line ~10), `data-testid="delete-project-button"` to the "Delete project" `<button>` (line ~13), and `data-testid="checks-empty"` to the "No checks yet." paragraph (line ~24). Keep existing attributes.

```html
<a class="btn" href="{{ base }}/projects/{{ project.id }}/checks/new" data-testid="new-check-link">New check</a>
```
```html
<button class="btn danger" type="submit" data-testid="delete-project-button">Delete project</button>
```
```html
<div class="cb"><p style="margin:0;color:var(--muted)" data-testid="checks-empty">No checks yet.</p></div>
```

- [ ] **Step 5: Edit `templates/dashboard.html`**

Add `data-testid="dashboard-empty"` to the "No projects yet." paragraph (line ~16). Keep existing attributes.

```html
<p data-testid="dashboard-empty">No projects yet. Create one to start watching a job.</p>
```

- [ ] **Step 6: Verify the testids are present**

Run:
```bash
rg -c 'data-testid' templates/project_form.html templates/check_form.html templates/check.html templates/project.html templates/dashboard.html
```
Expected: `project_form.html:2`, `check_form.html:3`, `check.html:7`, `project.html:3`, `dashboard.html:1`. (Counts are a sanity check, not exact contract — confirm each intended hook exists.)

- [ ] **Step 7: Rebuild the binary (askama compiles templates in)**

Run:
```bash
cargo build
```
Expected: builds successfully. A malformed template would fail here.

- [ ] **Step 8: Commit**

```bash
git add templates/project_form.html templates/check_form.html templates/check.html templates/project.html templates/dashboard.html
git commit -m "test: add data-testid hooks for core monitoring E2E"
```

---

### Task 2: Extend the E2E harness (base-url env + ping helper)

**Files:**
- Modify: `e2e/support/server.js`
- Modify: `e2e/support/api.js`

**Interfaces:**
- Consumes: existing `spawnPingward()` returning `{ url, dbPath, cleanup }`; existing `ApiHelper` constructed with `baseUrl`.
- Produces: `spawnPingward` now sets `PINGWARD_BASE_URL` to the server URL; `ApiHelper.ping(pingUrl, kind)` drives a success/fail ping.

This task has no dedicated unit test (the harness is exercised by the feature suite in Task 3). Verify by inspection + a successful `npx bddgen` (Task 3 gates the runtime behaviour).

- [ ] **Step 1: Add `PINGWARD_BASE_URL` to the spawned env**

In `e2e/support/server.js`, inside the `spawn(BINARY, [], { ... env: { ... } })` call, add `PINGWARD_BASE_URL: url` alongside the existing `DATABASE_URL` / `PINGWARD_BIND` / `RUST_LOG` entries.

```javascript
    env: {
      ...process.env,
      DATABASE_URL: `sqlite://${dbPath}?mode=rwc`,
      PINGWARD_BIND: `127.0.0.1:${port}`,
      PINGWARD_BASE_URL: url,
      RUST_LOG: "warn",
    },
```

- [ ] **Step 2: Add `ping()` to `ApiHelper`**

In `e2e/support/api.js`, add a method that pings the exact URL the UI renders. `kind` is `"success"` or `"fail"`.

```javascript
  // Drive a ping against the exact URL the check page renders. The ping
  // endpoints are public and CSRF-exempt; a success ping marks the check up,
  // a fail ping marks it down (both synchronous within the request).
  async ping(pingUrl, kind) {
    const target = kind === "fail" ? `${pingUrl}/fail` : pingUrl;
    const res = await fetch(target);
    if (!res.ok) {
      throw new Error(`ping (${kind}) failed: HTTP ${res.status}`);
    }
  }
```

- [ ] **Step 3: Commit**

```bash
git add e2e/support/server.js e2e/support/api.js
git commit -m "test: set base URL and add ping helper to E2E harness"
```

---

### Task 3: Monitoring feature file and step definitions

**Files:**
- Create: `e2e/features/monitoring.feature`
- Create: `e2e/steps/monitoring.steps.js`

**Interfaces:**
- Consumes: `test`/`expect` from `../support/fixtures.js`; the `api` fixture (with `bootstrapAdmin` and the new `ping`); the auth steps `an admin {string} with password {string} exists` and `I am signed in as {string} with password {string}` (already defined in `e2e/steps/auth.steps.js`); the `data-testid` hooks from Task 1; `PINGWARD_BASE_URL` set by Task 2.
- Produces: the runnable monitoring suite.

- [ ] **Step 1: Write the feature file**

Create `e2e/features/monitoring.feature` with exactly this content:

```gherkin
Feature: Monitoring core

  Background:
    Given an admin "admin" with password "correct horse" exists
    And I am signed in as "admin" with password "correct horse"

  Scenario: Create a project
    When I create a project named "Nightly jobs"
    Then I am on the project page for "Nightly jobs"

  Scenario: Create a check
    Given a project named "Nightly jobs"
    When I create a check named "backup" with period 60
    Then I am on the check page
    And the check status is "new"
    And the ping URL is shown

  Scenario: A success ping turns the check up
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I send a "success" ping
    And I reload the check page
    Then the check status is "up"

  Scenario: A fail ping turns the check down
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I send a "fail" ping
    And I reload the check page
    Then the check status is "down"

  Scenario: Acknowledge a down check
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    And I send a "fail" ping
    And I reload the check page
    When I acknowledge the check
    Then the acknowledge control is gone

  Scenario: Pause and resume a check
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I pause the check
    Then the check status is "paused"
    When I resume the check
    Then the check status is not "paused"

  Scenario: Regenerate the ping URL
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I regenerate the ping URL
    Then the ping URL is different from before

  Scenario: Delete a check
    Given a project named "Nightly jobs"
    And a check named "backup" with period 60
    When I delete the check
    Then the project has no checks

  Scenario: Delete a project
    Given a project named "Nightly jobs"
    When I delete the project
    Then the dashboard shows no projects
```

- [ ] **Step 2: Write the step definitions**

Create `e2e/steps/monitoring.steps.js` with exactly this content. Note the shared-state pattern: `savedPingUrl` is a module-scoped variable used only by the regenerate scenario (write then read within one scenario); because scenarios run in isolated worker contexts with a fresh page, this is safe, but keep it scoped to the single scenario's own steps.

```javascript
import { createBdd } from "playwright-bdd";
import { test, expect } from "../support/fixtures.js";

const { Given, When, Then } = createBdd(test);

// Navigate to the dashboard, then create a project via the "New project"
// flow. The project form lives at /projects/new and redirects to the project
// page on submit.
When("I create a project named {string}", async ({ page, serverUrl }, name) => {
  await page.goto(`${serverUrl}/projects/new`);
  await page.getByTestId("project-name-input").fill(name);
  await page.getByTestId("project-submit").click();
});

Then(
  "I am on the project page for {string}",
  async ({ page }, name) => {
    await expect(page).toHaveURL(/\/projects\/\d+$/);
    await expect(page.getByRole("heading", { name })).toBeVisible();
  }
);

// Create a project as a precondition and stay on its page (later steps read
// the "New check" link from it).
Given("a project named {string}", async ({ page, serverUrl }, name) => {
  await page.goto(`${serverUrl}/projects/new`);
  await page.getByTestId("project-name-input").fill(name);
  await page.getByTestId("project-submit").click();
  await expect(page).toHaveURL(/\/projects\/\d+$/);
});

// Create a check from the current project page. Period mode requires a
// positive period; grace/timezone are pre-filled by the form.
async function createCheck(page, name, period) {
  await page.getByTestId("new-check-link").click();
  await page.getByTestId("check-name-input").fill(name);
  await page.getByTestId("check-period-input").fill(String(period));
  await page.getByTestId("check-submit").click();
  await expect(page).toHaveURL(/\/checks\/\d+$/);
}

When(
  "I create a check named {string} with period {int}",
  async ({ page }, name, period) => {
    await createCheck(page, name, period);
  }
);

Given(
  "a check named {string} with period {int}",
  async ({ page }, name, period) => {
    await createCheck(page, name, period);
  }
);

Then("I am on the check page", async ({ page }) => {
  await expect(page).toHaveURL(/\/checks\/\d+$/);
});

Then("the check status is {string}", async ({ page }, status) => {
  await expect(page.getByTestId("check-status")).toHaveText(status);
});

Then("the check status is not {string}", async ({ page }, status) => {
  await expect(page.getByTestId("check-status")).not.toHaveText(status);
});

Then("the ping URL is shown", async ({ page }) => {
  await expect(page.getByTestId("ping-url")).toBeVisible();
});

// Read the URL the check page renders and drive a ping at it via the API
// helper. The page's rendered URL points at the test server because the
// harness sets PINGWARD_BASE_URL.
When("I send a {string} ping", async ({ page, api }, kind) => {
  const pingUrl = (await page.getByTestId("ping-url").textContent()).trim();
  await api.ping(pingUrl, kind);
});

When("I reload the check page", async ({ page }) => {
  await page.reload();
});

When("I acknowledge the check", async ({ page }) => {
  await page.getByTestId("ack-button").click();
});

Then("the acknowledge control is gone", async ({ page }) => {
  await expect(page.getByTestId("ack-button")).toHaveCount(0);
});

When("I pause the check", async ({ page }) => {
  await page.getByTestId("pause-button").click();
});

When("I resume the check", async ({ page }) => {
  await page.getByTestId("resume-button").click();
});

// Capture the current ping URL, regenerate, and confirm it changed.
When("I regenerate the ping URL", async ({ page }) => {
  const before = (await page.getByTestId("ping-url").textContent()).trim();
  await page.getByTestId("regenerate-button").click();
  await expect(page.getByTestId("ping-url")).not.toHaveText(before);
});

Then("the ping URL is different from before", async ({ page }) => {
  // The assertion is performed in the When step (the before-value is only in
  // scope there); here we simply confirm a ping URL is still present.
  await expect(page.getByTestId("ping-url")).toBeVisible();
});

// Delete flows submit through a confirm() dialog; accept it.
When("I delete the check", async ({ page }) => {
  page.on("dialog", (d) => d.accept());
  await page.getByTestId("delete-check-button").click();
  await expect(page).toHaveURL(/\/projects\/\d+$/);
});

Then("the project has no checks", async ({ page }) => {
  await expect(page.getByTestId("checks-empty")).toBeVisible();
});

When("I delete the project", async ({ page }) => {
  page.on("dialog", (d) => d.accept());
  await page.getByTestId("delete-project-button").click();
  await expect(page).toHaveURL(/\/$/);
});

Then("the dashboard shows no projects", async ({ page }) => {
  await expect(page.getByTestId("dashboard-empty")).toBeVisible();
});
```

- [ ] **Step 3: Generate the BDD test files**

Run:
```bash
cd e2e && npx bddgen
```
Expected: succeeds, generating spec files for `monitoring.feature` with no "undefined step" warnings. If any step is reported undefined, fix the step definition before proceeding.

- [ ] **Step 4: Run the monitoring feature**

Ensure the binary is current (Task 1 rebuilt it; if templates changed since, rebuild). Run:
```bash
cd e2e && npx playwright test monitoring
```
Expected: all 9 scenarios pass.

- [ ] **Step 5: Run the full E2E suite (auth + monitoring) to confirm no regression**

```bash
cd e2e && npx bddgen && npx playwright test
```
Expected: all scenarios (5 auth + 9 monitoring) pass.

- [ ] **Step 6: Commit**

```bash
git add e2e/features/monitoring.feature e2e/steps/monitoring.steps.js
git commit -m "test: add core monitoring E2E feature and steps"
```

---

## Deviations / Notes

- The `Then "the ping URL is different from before"` step is a thin presence
  check because the actual before/after comparison must happen in the `When`
  step that holds the captured value. This keeps the Gherkin readable while
  the real assertion runs where the state is in scope. An implementer MAY
  instead move the comparison entirely into the `When` and make the feature's
  `Then` line a pure presence assertion — either is acceptable as long as the
  regeneration is actually asserted.
- No CI change is required: the existing sharded `e2e-tests` job runs `bddgen`
  then the whole suite, so `monitoring.feature` is picked up automatically.
