# Admin Follow-ups — Design

Date: 2026-07-14
Status: Approved (design); implementation pending
Branch: `fix/admin-followups` (off `main` after PR #23 merged)

Addresses the five follow-ups recorded on PR #23 (admin feature set). Four are
small, contained fixes; the fifth (CSRF) is a cross-cutting security feature.

## 1. Small fixes

### F1 — Admin nav link on validation-error re-renders
`check_create_core`, `check_update_core`, and `channel_create_core` re-render
their form on validation error with `is_admin` tied to the URL-prefix `admin`
flag, so an admin submitting an invalid form via the owner path transiently
loses the Admin nav link on that error page. Thread the viewer's `is_admin`
into those cores (owner callers pass `user.is_admin`, admin callers pass
`true`), the same split already applied to the render helpers in `f28ce41`.

### F2 — No audit row for non-existent user targets
`users_delete` and `users_set_password` write an audit row even when the target
id does not exist (the UPDATE/DELETE is a silent no-op). Gate both: resolve the
target with `find_user_by_id` first; if `None`, redirect without mutating or
auditing. (`users_delete` already loads the target for its guard — reuse it.)

### F3 — Delete lockout guard consistency
`users_delete` counts the last admin via `list_users().filter(is_admin).count()`
while `users_toggle_admin`/`users_set_disabled` use `count_enabled_admins()`.
Switch `users_delete` to `count_enabled_admins()` so all three lockout guards
count the same population (enabled admins). Behaviourally: refuse to delete a
user who is the last enabled admin.

### F4 — Cross-engine NULL ordering
`list_down_checks_with_owner` uses `ORDER BY c.last_ping_at`; SQLite sorts NULLs
first, PostgreSQL sorts them last, so never-pinged down checks land at opposite
ends per backend. Make it deterministic on both:
`ORDER BY c.last_ping_at IS NULL, c.last_ping_at` (the `IS NULL` boolean sorts
NULLs consistently last on both engines).

## 2. CSRF protection (F5) — synchronizer token, template-embedded

### Approach
Per-session synchronizer token stored server-side, embedded as a hidden field
in every browser POST form, validated by middleware. No JS dependency, no new
server secret.

### Storage
- Migration `0007_session_csrf`: add `csrf_token TEXT` to `sessions` (SQLite +
  PostgreSQL parity; `NOT NULL DEFAULT ''` so existing rows are valid, though
  sessions are short-lived).
- `start_session` generates a token (`new_session_token()` / a fresh UUID) and
  stores it alongside the session row. `create_session` gains a `csrf_token`
  parameter.
- Store: `session_csrf_token(session_id) -> Option<String>` to look it up
  (used by the validation layer and by the render path to embed it).

### Validation — middleware scoped to `web::routes()` only
- A `csrf` middleware layer is applied to `web::routes()` **only**. `ping::routes()`
  (machine check-in endpoints hit by curl/cron), `assets::routes()`, and
  `/healthz` are in separate routers merged in `app()` and are therefore never
  subject to CSRF — the exemption is structural, not a path allowlist.
- The middleware acts only on unsafe methods (POST). For those it:
  1. Skips exempt pre-session paths: `POST /login`, `POST /setup` (no session
     exists yet). Everything else under `web::routes()` is protected (incl.
     `/logout` and all admin/user/project/check/channel mutations).
  2. Resolves the caller's session token from the `pingward_session` cookie and
     looks up its `csrf_token`. No session / no token → 403.
  3. Reads the submitted token from EITHER the `_csrf` form field OR the
     `X-CSRF-Token` request header. Constant-time compare to the session token;
     mismatch → 403.
- Body handling: to read the `_csrf` form field the middleware buffers the
  request body, parses `application/x-www-form-urlencoded`, then reconstructs
  the request with the buffered body so the downstream handler's `Form<T>`
  extractor still works. (The header path avoids buffering; tests use it.)

### Token emission in templates
- Thread a `csrf: String` field through every base-extending template struct
  that renders a protected POST form (parallel to the existing `show_nav`
  field). Add `<input type="hidden" name="_csrf" value="{{ csrf }}">` to each
  such `<form method="post">`.
- Pre-session forms (`login.html`, `setup.html`) are exempt and carry no token
  (empty `csrf`).
- The render/handler path supplies the token by looking up the current
  session's `csrf_token` (from the session cookie) when building the template.

### Test strategy (contain the blast radius)
Adding CSRF to `web::routes()` would 403 every existing web POST test. To
localize the change:
- The middleware accepts the token via the `X-CSRF-Token` header (above).
- Each integration test file's server helper (`server()` / `logged_in_server()`
  / `admin_server()` in `tests/`) — after establishing a session — reads that
  session's `csrf_token` from the store and configures the `TestServer` to send
  it as a default `X-CSRF-Token` header on all subsequent requests.
- New focused tests: (a) a protected POST without a token → 403; (b) with a
  valid token → success; (c) `POST /ping/{uuid}` still works with no token
  (ping router unaffected); (d) `POST /login` works with no token (exempt).

## 3. Scope / sequencing

One branch, one PR. Suggested order: F2/F3 (store+handler, tiny) → F1 (nav
threading) → F4 (SQL) → F5 (CSRF: migration → session token → middleware →
template threading → test-helper updates → new CSRF tests). CSRF lands last
because it changes the test harness contract.

## 4. Out of scope
- CSRF for the machine `/ping/*` endpoints (they authenticate by UUID, are not
  browser-driven, and must remain curl/cron-callable).
- Rotating the CSRF token per request (per-session token is sufficient here).
- Audit-log viewer UI and other items already deferred from PR #23.
