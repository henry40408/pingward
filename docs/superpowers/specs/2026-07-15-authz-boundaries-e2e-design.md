# Authorization & Security Boundaries E2E — Design

**Date:** 2026-07-15
**Status:** Approved
**Builds on:** [2026-07-15-e2e-testing-harness-design.md](2026-07-15-e2e-testing-harness-design.md) (auth batch, PR #30) and [2026-07-15-core-monitoring-e2e-design.md](2026-07-15-core-monitoring-e2e-design.md) (monitoring batch, PR #31)

## 1. Goal

Extend the browser-level E2E suite to cover pingward's authorization and
security boundaries — the guards that keep one user out of another user's
data, keep non-admins out of the admin surface, and reject unauthenticated or
forged state-changing requests. These are regression nets for behavior added
in PRs #24/#25 (CSRF) and #27 (admin-only nav), plus the standing
`owned_project` and `AdminUser`/`CurrentUser` guards.

## 2. Scope

A new `e2e/features/authz.feature` with six scenarios (positive controls
included so an always-pass assertion cannot hide a regression):

1. An admin **sees** the Settings and Admin nav links (positive control)
2. A non-admin does **not** see the Settings or Admin nav links (#27 regression)
3. A non-admin is **forbidden** from `/admin` → `403`
4. A non-admin **cannot read** another user's project → `404`
5. A **logged-out** visitor hitting `/` is redirected to `/login`
6. A state-changing **POST without a CSRF token** is rejected → `403` (#24/#25 regression)

### Out of scope (future batches)

- Positive CSRF path (a valid form submit succeeds) — already exercised
  implicitly by every monitoring scenario.
- Admin cross-user *success* flows (`/admin/*` acting on another user's data)
  and the audit log — that is the dedicated admin batch (#7 in the roadmap).
- Disabled-user login rejection — belongs with the user-management batch (#6).
- Time-dependent states.

## 3. Domain facts the design relies on

Verified against source before writing this spec:

- **Unauthenticated access** (`src/auth.rs::CurrentUser`): a missing/invalid
  session yields `Err(Redirect::to("/login"))`. So a logged-out `GET /`
  lands on `/login` (final response 200 at `/login`).
- **Admin guard** (`src/auth.rs::AdminUser`): a non-admin yields
  `Err((StatusCode::FORBIDDEN, "admin only"))`. So a non-admin `GET /admin`
  returns **403**.
- **Ownership guard** (`src/web.rs::owned_project`): when
  `project.user_id != user_id` it returns `AppError::NotFound`, which
  `src/error.rs` renders as **404 "not found"** — deliberately hiding the
  resource's existence rather than returning 403. So a non-owner (even an
  admin, via the non-admin `/projects/{id}` route) gets **404**, not 403.
- **Nav gating** (`templates/base.html`): the Settings and Admin links are
  both wrapped in a single `{% if is_admin %}` — a non-admin sees neither.
- **CSRF guard** (`src/web.rs::csrf_guard`): applied to the `web` router only.
  Safe methods (GET/HEAD/OPTIONS) and the pre-session `POST /login`, `POST
  /setup` pass through. Every other state-changing request must present the
  session's stored token via `X-CSRF-Token` header or `_csrf` form field; a
  missing/mismatched token (or missing session) returns **403**.
- **User creation** (`src/web.rs::users_create`, `templates/users.html`): an
  admin POSTs `/users` with `username` + `password`; the `is_admin` checkbox
  is unchecked by default, so the created user is a **non-admin**. The form
  lives at `templates/users.html` (`#username`, `#password`, submit
  "Create user").

## 4. Harness changes

All additive.

1. **`templates/users.html`** — add `data-testid` to the create-user form:
   `user-username-input` (the `#username` input), `user-password-input` (the
   `#password` input), `user-submit` (the "Create user" button).
2. **`templates/base.html`** — add `data-testid="nav-settings"` and
   `data-testid="nav-admin"` to the two admin-only nav links, so their
   presence/absence is asserted cleanly (rather than by fragile link text).
3. **`e2e/support/fixtures.js`** — add a scenario-scoped `world` fixture (a
   plain mutable object, fresh per test) used to carry state across steps: the
   remembered project URL and the most recent captured HTTP response status.

No application (`src/*`) behavior changes — additive test hooks only.

## 5. Selector additions (app templates)

| Template | `data-testid` |
|----------|---------------|
| `users.html` | `user-username-input`, `user-password-input`, `user-submit` |
| `base.html` | `nav-settings`, `nav-admin` |

## 6. New step definitions (`e2e/steps/authz.steps.js`)

Reuses auth steps (`I am signed in as …`, `I sign out`, `I am on the login
page`) and the monitoring `I create a project named …` step. New steps:

- `Given a non-admin user {string} with password {string} exists` — assumes
  the admin is signed in; navigates to `/users`, fills the create-user form
  (leaving `is_admin` unchecked), submits.
- `Then the {string} nav link is visible` / `is not visible` — maps the label
  ("Settings"/"Admin") to its testid (`nav-settings`/`nav-admin`) and asserts
  `toBeVisible()` / `toHaveCount(0)`.
- `When I navigate to {string}` — `page.goto(serverUrl + path)`, storing the
  response status on `world`.
- `Then the response status is {int}` — asserts `world.status`.
- `When I POST to {string} without a CSRF token` — issues
  `page.request.post(serverUrl + path, { form: {} })` (the browser context's
  session cookie rides along; no `_csrf`), storing the status on `world`.
- `When I remember the current project` — records `page.url()` on `world`.
- `When I revisit it as {string} with password {string}` — signs out, signs in
  as the named user, `page.goto(world.projectUrl)`, stores the status.

## 7. Feature file (behavioural spec)

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

## 8. Testing strategy

- Status-code scenarios (3, 4, 6) assert the real HTTP response from
  `page.goto` / `page.request`, which is deterministic and free of any
  scheduler/clock dependence.
- Scenario 5 asserts by final URL (`/login`) rather than status, because the
  redirect resolves to a 200 login page.
- Scenario 6 is the only non-UI-click step: a raw authenticated request that
  targets the CSRF middleware directly rather than a rendered form (rendered
  forms always carry `_csrf`, so the browser cannot naturally omit it).
- Positive controls (scenario 1; the admin's own view) guard against an
  assertion that would pass even if the gating were removed.
- Reuses Approach A isolation, playwright-bdd, and `data-testid`. The existing
  sharded `e2e-tests` CI job runs `bddgen` then the whole suite, so
  `authz.feature` is picked up automatically with no CI change.
