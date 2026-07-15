# Core Monitoring E2E — Design

**Date:** 2026-07-15
**Status:** Approved
**Builds on:** [2026-07-15-e2e-testing-harness-design.md](2026-07-15-e2e-testing-harness-design.md) (auth batch, PR #30)

## 1. Goal

Extend the browser-level E2E suite to cover pingward's monitoring core: an
owned (non-admin) user managing the full lifecycle **Project → Check → Ping →
status**. This is the heart of the product — a check exposes a ping URL, and
incoming pings drive its status (`new`/`up`/`down`/`paused`), which the UI
reflects.

## 2. Scope

Covered flows, all in a new `e2e/features/monitoring.feature`:

1. Create a project → land on the project page
2. Create a check (period schedule) → land on the check page, status `new`,
   ping URL visible
3. Success ping turns the check **up**
4. Fail ping turns the check **down**
5. Acknowledge a **down** check
6. Pause and resume a check
7. Regenerate the ping URL
8. Delete a check
9. Delete a project

### Out of scope (future batches)

- Notification channels (SMTP/webhook side effects)
- Settings page and user management
- Admin cross-user surfaces (`/admin/*`) — see the planned admin design
- Non-deterministic display states (`late`, overrun) that require the
  scheduler / wall-clock time to advance

## 3. Reuse of the existing harness

The auth batch established the harness; this batch reuses it unchanged in
approach:

- **Isolation (Approach A):** each scenario spawns a fresh compiled
  `pingward` binary against a fresh temp SQLite DB. The Playwright
  `pingwardServer` fixture is function-scoped, so every scenario is isolated.
- **Bootstrap:** each scenario needs an authenticated session. A Gherkin
  `Background` bootstraps the first admin via the CSRF-exempt `POST /setup`
  (`ApiHelper.bootstrapAdmin`) and signs in through the browser, reusing the
  auth batch's existing step definitions.
- **Selectors:** stable `data-testid` hooks, matching the auth batch
  convention. Existing `id` attributes are kept for label association.

## 4. Domain facts the design relies on

Verified against source before writing this spec:

- **Stored statuses** (`src/models.rs`): `new`, `up`, `down`, `paused`.
  A new check is `new`.
- **Ping effects** (`src/ping.rs::apply`): a `success` ping sets the check
  `Up` immediately and synchronously; a `fail` ping sets it `Down`
  immediately. A **paused** check records the ping but is not resurrected.
- **Display status** (`src/view.rs::display_status`): for `new`/`down`/
  `paused` the badge equals the stored status. For `up`, the badge is `up`
  unless the check is inside its grace window (`up` → `late`). With
  `due = last_ping + period + grace`, the grace window opens at
  `last_ping + period`; immediately after a success ping `now < last_ping +
  period` for any positive period, so the badge is a stable `up`. Scenarios
  use `period_secs = 60`, avoiding any `late` flakiness.
- **Check form** (`src/web.rs::empty_check_form` / `validate_check`): default
  schedule kind is `period`; `period_secs` is empty and **required** in period
  mode; `grace_secs` (300) and `timezone` (UTC) are pre-filled. So creating a
  check requires filling `name` **and** `period_secs`.
- **Ping URL** (`src/web.rs::render_check_page`): rendered as
  `{base_url}/ping/{uuid}` where `base_url` comes from `PINGWARD_BASE_URL`
  (default `http://localhost:8080`). The check page shows the URL in a
  `<code>` element.
- **Acknowledge** (`templates/check.html`): the Acknowledge control is
  rendered only when `status == "down" && !acknowledged`.
- **Delete confirmations**: deleting a check (check page) and a project
  (project page) submit through an `onsubmit="return confirm(...)"` dialog.
- **Regenerate** (`src/web.rs::check_regenerate`): assigns a new `ping_uuid`,
  so the rendered ping URL changes.

## 5. Harness changes

Two small, additive changes to the existing support code:

1. **`e2e/support/server.js` — `spawnPingward`:** add
   `PINGWARD_BASE_URL: url` to the child process env. Without this the check
   page renders `http://localhost:8080/ping/...`, which does not reach the
   ephemeral test server. With it, the rendered URL targets the live test
   server and the suite can ping the exact URL the UI shows the user.

2. **`e2e/support/api.js` — `ApiHelper`:** add
   `async ping(pingUrl, kind)` that issues `GET pingUrl` for `kind ==
   "success"` and `GET ${pingUrl}/fail` for `kind == "fail"`, throwing on a
   non-OK response. Used by steps that read the ping URL from the page and
   then drive a ping.

Confirm-dialog handling lives in the step definitions: before submitting a
delete, register `page.on("dialog", (d) => d.accept())`.

## 6. Selector additions (app templates)

All additive `data-testid` attributes; existing `id`s and markup are kept.

| Template | `data-testid` |
|----------|---------------|
| `project_form.html` | `project-name-input`, `project-submit` |
| `check_form.html` | `check-name-input`, `check-period-input`, `check-submit` |
| `check.html` | `check-status` (badge), `ping-url` (code), `ack-button`, `pause-button`, `resume-button`, `regenerate-button`, `delete-check-button` |
| `project.html` | `new-check-link`, `delete-project-button`, `checks-empty` (the "No checks yet." state) |
| `dashboard.html` | `dashboard-empty` (the "No projects yet." state) |

`pause-button` and `resume-button` are on mutually exclusive branches of the
check page (a check shows exactly one), so both hooks never coexist.

## 7. Feature file (behavioural spec)

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

## 8. Testing strategy

- Every scenario is a real browser session against a real binary; assertions
  read rendered DOM state via `data-testid`.
- Status transitions are deterministic (synchronous within the ping request),
  so no polling/waiting on the scheduler is needed. Steps reload the page and
  assert the badge.
- Pings target the exact URL the UI renders, so the test also implicitly
  verifies ping-URL rendering.
- CI: the existing sharded `e2e-tests` job runs `bddgen` then the whole
  `e2e/` suite; the new feature is picked up automatically with no CI change.
