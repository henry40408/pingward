# Admin Follow-ups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve the five PR #23 follow-ups: four small fixes plus CSRF protection for browser POST routes.

**Architecture:** F1–F4 are contained fixes in `src/web.rs`/`src/store.rs`. F5 adds a per-session synchronizer CSRF token (new `sessions.csrf_token` column), a middleware applied only to `web::routes()`, hidden `_csrf` inputs in forms, and a header escape hatch (`X-CSRF-Token`) so integration tests stay maintainable.

**Tech Stack:** Rust, axum 0.8, sqlx 0.9 `Any` (SQLite + PostgreSQL), askama, argon2, chrono. Tests via `cargo nextest run` with `axum-test`.

## Global Constraints

- Branch `fix/admin-followups` (off `main`, already checked out). Do NOT create another branch. Commit directly on it.
- sqlx `Any`: `$N` placeholders, `RETURNING id`; migrations in SQLite+PostgreSQL parity; boolean/text conventions per existing schema; timestamps RFC3339 TEXT.
- `cargo nextest run` (never `cargo test`); `cargo fmt` before every commit; `cargo clippy --all-targets -- -D warnings` clean before committing; GPG-signed commits (no `--no-gpg-sign`), message trailer `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`.
- Stage files explicitly by name; never `git add -A`/`git add .`.
- The machine ping endpoints (`ping::routes()` — `/ping/{uuid}...`), `assets::routes()`, and `/healthz` MUST remain reachable without CSRF (they are separate routers; do not add CSRF to them).
- Live PostgreSQL for parity runs: `container ls` → read `pingward-pg` IP → `export TEST_DATABASE_URL=postgres://postgres:postgres@<ip>:5432/postgres`.

---

## File Structure

- `src/web.rs` — F1 (3 core fns), F2/F3 (users_delete/users_set_password), F5 (`start_session` token, csrf middleware module, template `csrf` fields, form token supply).
- `src/store.rs` — F3 (delete guard), F4 (ORDER BY), F5 (`create_session` param, `session_csrf_token`).
- `src/auth.rs` — F5 (expose a csrf helper if needed; SESSION_COOKIE reuse).
- `migrations/{sqlite,postgres}/0007_session_csrf.sql` — F5.
- `templates/*.html` — F5 (hidden `_csrf` inputs).
- `tests/*.rs` — updated server helpers + new tests.

---

## Task 1: F2 + F3 — no-op audit gate + delete guard consistency

**Files:** Modify `src/web.rs` (`users_delete`, `users_set_password`), `src/store.rs` (none new — reuse `count_enabled_admins`). Test: `tests/users_admin.rs`.

**Interfaces:** Consumes `Store::{find_user_by_id, count_enabled_admins, delete_user, set_user_password, record_audit}`.

- [ ] **Step 1: Write failing tests** in `tests/users_admin.rs` (reuse `admin_server()`):
```rust
#[tokio::test]
async fn deleting_nonexistent_user_writes_no_audit() {
    let (server, store, _admin) = admin_server().await;
    let before = store.list_audit(50).await.unwrap().len();
    server.post("/users/99999/delete").await; // nonexistent id
    let after = store.list_audit(50).await.unwrap();
    assert!(!after.iter().any(|a| a.action == "user.delete" && a.target_id == Some(99999)));
    assert_eq!(after.len(), before);
}

#[tokio::test]
async fn resetting_password_for_nonexistent_user_writes_no_audit() {
    let (server, store, _admin) = admin_server().await;
    server.post("/users/99999/password").form(&[("password", "whatever12")]).await;
    assert!(!store.list_audit(50).await.unwrap().iter()
        .any(|a| a.action == "user.password_reset" && a.target_id == Some(99999)));
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run --test users_admin deleting_nonexistent_user_writes_no_audit resetting_password_for_nonexistent_user_writes_no_audit`
Expected: FAIL (audit rows currently written for nonexistent ids).

- [ ] **Step 3: Fix `users_delete`** — restructure so delete+audit run only when the target exists, and use `count_enabled_admins()`:
```rust
async fn users_delete(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    if id == admin.id {
        return Ok(Redirect::to("/users").into_response());
    }
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/users").into_response());
    };
    // Refuse to delete the last enabled admin.
    if target.is_admin && !target.disabled && state.store.count_enabled_admins().await? <= 1 {
        return Ok(Redirect::to("/users").into_response());
    }
    state.store.delete_user(id).await?;
    state.store.record_audit(
        &crate::store::NewAudit {
            actor_user_id: admin.id,
            actor_username: &admin.username,
            action: "user.delete",
            target_type: Some("user"),
            target_id: Some(id),
            ..Default::default()
        },
        Utc::now(),
    ).await?;
    Ok(Redirect::to("/users").into_response())
}
```

- [ ] **Step 4: Fix `users_set_password`** — gate on existence:
```rust
    if form.password.is_empty() {
        return Ok(Redirect::to("/users").into_response());
    }
    if state.store.find_user_by_id(id).await?.is_none() {
        return Ok(Redirect::to("/users").into_response());
    }
    let phc = hash_password(&form.password).map_err(|e| AppError::Other(e.to_string().into()))?;
    // ... existing set_user_password + record_audit unchanged ...
```

- [ ] **Step 5: Run tests + existing guard tests**

Run: `cargo nextest run --test users_admin`
Expected: PASS (new tests + existing `cannot_demote_last_admin` etc. still green; note the delete guard now uses enabled-admin count — the existing `create_and_delete`/lockout tests must still pass).

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/web.rs tests/users_admin.rs
git commit -m "fix: skip audit/mutation for nonexistent user targets and align delete lockout guard with count_enabled_admins"
```

---

## Task 2: F1 — Admin nav link on validation-error re-renders

**Files:** Modify `src/web.rs` (`check_create_core`, `check_update_core`, `channel_create_core`, and `empty_check_form` if it constructs the form template). Test: `tests/admin.rs` or `tests/forms_view.rs`.

**Interfaces:** These cores are called from both owner wrappers (have `CurrentUser(user)`) and admin wrappers (have `AdminUser`, viewer is admin).

- [ ] **Step 1: Locate the error re-render sites.** Grep `is_admin: admin` inside `check_create_core`, `check_update_core`, `channel_create_core`, and `empty_check_form`. These set the form template's `is_admin` (nav) from the URL-prefix `admin` bool.

- [ ] **Step 2: Write a failing test** (add to `tests/admin.rs`, reuse `admin_server()`): submit an INVALID check-create form via the OWNER route as an admin, assert the error re-render body still contains `href="/admin"`:
```rust
#[tokio::test]
async fn admin_keeps_nav_link_on_owner_form_validation_error() {
    let (server, store, admin_id) = admin_server().await;
    let pid = store.create_project(admin_id, "p", None, None, chrono::Utc::now()).await.unwrap();
    // invalid: blank name (or whatever triggers the validation-error re-render)
    let res = server.post(&format!("/projects/{pid}/checks"))
        .form(&[("name", ""), ("schedule_kind", "period"), ("period_secs", "60"), ("grace_secs", "30")])
        .await;
    // error re-render is 200 with the form; must still show the Admin nav link
    assert!(res.text().contains("href=\"/admin\""));
}
```
Adjust the form fields to whatever actually triggers `check_create_core`'s validation-error branch (read the handler; use the minimal invalid input).

- [ ] **Step 3: Run to verify it fails**

Run: `cargo nextest run --test admin admin_keeps_nav_link_on_owner_form_validation_error`
Expected: FAIL (error re-render sets `is_admin` from `admin=false`, so no nav link).

- [ ] **Step 4: Thread the viewer's admin status into the cores.** Add an `is_admin: bool` parameter to `check_create_core`, `check_update_core`, `channel_create_core` (and `empty_check_form` if used by them), and set the form template's `is_admin` field from that param instead of the URL-prefix `admin`. Update callers: owner wrappers pass `user.is_admin`; admin wrappers pass `true`. (Same split already applied to `render_project_page`/`render_check_page` in commit `f28ce41` — mirror it.)

- [ ] **Step 5: Run test + full check-form tests**

Run: `cargo nextest run --test admin --test forms_view`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/web.rs tests/admin.rs
git commit -m "fix: keep Admin nav link on form validation-error re-renders (viewer-scoped is_admin)"
```

---

## Task 3: F4 — deterministic cross-engine NULL ordering

**Files:** Modify `src/store.rs` (`list_down_checks_with_owner`). Test: `src/store.rs` inline tests.

- [ ] **Step 1: Change the ORDER BY.** In `list_down_checks_with_owner`, replace `ORDER BY c.last_ping_at` with `ORDER BY c.last_ping_at IS NULL, c.last_ping_at` (NULLs sort last consistently on both engines; add `, c.id` as a final tiebreaker for determinism).

- [ ] **Step 2: Write/adjust a test** in `src/store.rs` tests (use `seeded()`): create two down checks — one with a `last_ping_at`, one without (never pinged) — and assert `list_down_checks_with_owner()` returns the pinged one before the never-pinged one (NULL last). To set a check `down` + set/clear `last_ping_at`, mirror how neighbouring store tests manipulate check status/timestamps (e.g. `set_status`, or a direct UPDATE in the test).
```rust
#[tokio::test]
async fn down_checks_order_never_pinged_last() {
    let store = seeded().await;
    // ... create a project, two checks, mark both down; give check A a last_ping_at, leave B NULL ...
    let rows = store.list_down_checks_with_owner().await.unwrap();
    // A (pinged) precedes B (never pinged)
    let names: Vec<_> = rows.iter().map(|(c,_,_)| c.name.clone()).collect();
    let ia = names.iter().position(|n| n == "A").unwrap();
    let ib = names.iter().position(|n| n == "B").unwrap();
    assert!(ia < ib);
}
```

- [ ] **Step 3: Run**

Run: `cargo nextest run -p pingward store::tests::down_checks_order_never_pinged_last`
Expected: PASS on SQLite. (Cross-engine behavior verified in Task 6's PG run.)

- [ ] **Step 4: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/store.rs
git commit -m "fix: order down-checks with never-pinged last on both SQLite and PostgreSQL"
```

---

## Task 4: F5a — CSRF token + middleware + test-harness update

**Files:** Create `migrations/{sqlite,postgres}/0007_session_csrf.sql`. Modify `src/store.rs` (`create_session`, `session_csrf_token`), `src/web.rs` (`start_session`, csrf middleware), `src/lib.rs` (`app()` layering). Update ALL `tests/*.rs` server helpers. New CSRF tests.

**Interfaces:**
- Produces: `Store::create_session(id, user_id, csrf_token, expires_at)`, `Store::session_csrf_token(session_id) -> Result<Option<String>, _>`, a `web::csrf_guard` middleware, and a `sessions.csrf_token` column.

- [ ] **Step 1: Migrations**

`migrations/sqlite/0007_session_csrf.sql`:
```sql
ALTER TABLE sessions ADD COLUMN csrf_token TEXT NOT NULL DEFAULT '';
```
`migrations/postgres/0007_session_csrf.sql`:
```sql
ALTER TABLE sessions ADD COLUMN csrf_token TEXT NOT NULL DEFAULT '';
```

- [ ] **Step 2: Store — `create_session` gains `csrf_token`; add lookup.**
```rust
    pub async fn create_session(
        &self,
        id: &str,
        user_id: i64,
        csrf_token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO sessions (id, user_id, csrf_token, expires_at) VALUES ($1,$2,$3,$4)")
            .bind(id).bind(user_id).bind(csrf_token).bind(expires_at.to_rfc3339())
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn session_csrf_token(&self, session_id: &str) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT csrf_token FROM sessions WHERE id = $1")
            .bind(session_id).fetch_optional(&self.pool).await
    }
```
Update the existing `create_session` caller(s) — `start_session` in `web.rs` — and any test/store-internal callers of `create_session` to pass a token (tests can pass any non-empty string).

- [ ] **Step 3: `start_session` generates and stores a csrf token, and returns it** so the caller/render path can use it. Change its signature to also return the token (e.g. `-> Result<(CookieJar, String), AppError>`), or store it and let the render path look it up via `session_csrf_token`. Simplest: generate `let csrf = new_session_token();`, pass to `create_session`, and rely on `session_csrf_token` lookups elsewhere. Keep the session cookie exactly as today.

- [ ] **Step 4: CSRF middleware** (new, in `src/web.rs` or a `web` submodule). `async fn csrf_guard(State(state): State<AppState>, req: Request, next: Next) -> Response`:
  - If method is not state-changing (GET/HEAD/OPTIONS), call `next.run(req)` unchanged.
  - If the path is exempt (`/login`, `/setup`), pass through.
  - Resolve session id from the `pingward_session` cookie (parse `req.headers()` cookies). Look up `session_csrf_token`. Missing session/token → `StatusCode::FORBIDDEN`.
  - Read the submitted token: first check the `X-CSRF-Token` header. If absent, buffer the body (`axum::body::to_bytes` with a sane limit), parse `application/x-www-form-urlencoded` for `_csrf`, then rebuild the request with the buffered bytes so the downstream `Form<T>` extractor still works. Use `serde_urlencoded`/`form_urlencoded` to read just `_csrf` without consuming typed structs.
  - Constant-time compare submitted vs stored (e.g. lengths equal + `subtle`-style compare, or a simple byte compare — a timing side-channel on a random 36-char UUID is not a practical concern; a plain `==` is acceptable here, note it).
  - Mismatch → 403; match → `next.run(rebuilt_req)`.

- [ ] **Step 5: Apply the middleware to `web::routes()` ONLY** in `app()`:
```rust
pub fn app(state: AppState) -> Router {
    let web = web::routes().layer(axum::middleware::from_fn_with_state(state.clone(), web::csrf_guard));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(web)
        .merge(ping::routes())
        .merge(assets::routes())
        .with_state(state)
}
```
Export `csrf_guard` from `web` (make it `pub`). Confirm `from_fn_with_state` is available in axum 0.8 (`axum::middleware::from_fn_with_state`).

- [ ] **Step 6: Update EVERY test server helper to send the token as a header.** In each `tests/*.rs` that builds a `TestServer` and logs in (`server()`, `logged_in_server()`, `admin_server()` in `auth_web.rs`, `admin.rs`, `users_admin.rs`, `admin_dashboard.rs`, and any others), AFTER the session is established, read the session's csrf token and set it as a default header. Pattern:
```rust
// after login/setup established the session cookie + row:
let sid = /* the session id: query the single sessions row */
    sqlx::query_scalar::<_, String>("SELECT id FROM sessions LIMIT 1")
        .fetch_one(/* store pool — expose via a small store helper or reuse an existing one */).await.unwrap();
let csrf = store.session_csrf_token(&sid).await.unwrap().unwrap();
server.add_header(http::header::HeaderName::from_static("x-csrf-token"), http::HeaderValue::from_str(&csrf).unwrap());
```
Confirm the axum-test 21 method name for a persistent default header (likely `TestServer::add_header`). If a store pool accessor isn't public, add a tiny `Store` helper `first_session_id()` for tests, or read via an existing query. Keep this helper logic DRY per test file.

- [ ] **Step 7: New CSRF tests** (in `tests/admin.rs` or a new `tests/csrf.rs`):
```rust
// (a) protected POST without token → 403
async fn post_without_csrf_is_forbidden() { /* build logged-in server WITHOUT the header, POST /logout or /projects, assert 403 */ }
// (b) with valid header token → not 403 (redirect/200)
async fn post_with_csrf_header_succeeds() { /* uses the helper header, asserts 303/200 */ }
// (c) ping endpoint unaffected
async fn ping_post_needs_no_csrf() { /* POST /ping/{uuid} with no token → not 403 (200/appropriate) */ }
// (d) login exempt
async fn login_post_needs_no_csrf() { /* POST /login with no token → not 403 */ }
```

- [ ] **Step 8: Run the full suite**

Run: `cargo nextest run`
Expected: ALL green (every existing POST test now carries the header via its helper; new CSRF tests pass). Fix any test helper that missed the header (symptom: that file's POST tests 403).

- [ ] **Step 9: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add migrations/sqlite/0007_session_csrf.sql migrations/postgres/0007_session_csrf.sql src/store.rs src/web.rs src/lib.rs tests/*.rs
git commit -m "feat: CSRF protection for browser POST routes (per-session token, header/form, web-router-only)"
```

---

## Task 5: F5b — hidden `_csrf` inputs in forms

**Files:** Modify `src/web.rs` (thread `csrf: String` into base-extending template structs that render protected forms; supply the token in each render path) and `templates/*.html` (add hidden input to each protected `<form method="post">`). Test: a render test asserting the input is present + one end-to-end form-field POST.

- [ ] **Step 1: Thread a `csrf: String` field** into each template struct whose template contains a protected POST form (parallel to `show_nav`): dashboard, project, check, project_form, check_form, channel_form, settings, users, admin_projects, admin_dashboard (only those with forms). Do NOT add it to `login`/`setup` (exempt). The render path supplies the token by looking up the current session's `csrf_token` (resolve the session id from the request cookie via a `CookieJar`/`OptionalUser`-adjacent path, then `store.session_csrf_token`). Provide a small helper `current_csrf(state, jar) -> String` returning the token or `""`.

- [ ] **Step 2: Add the hidden input** to every protected `<form method="post" ...>` in the affected templates:
```html
<input type="hidden" name="_csrf" value="{{ csrf }}">
```
Grep each template for `method="post"` and add the input inside each such form. (Skip `login.html`/`setup.html`.)

- [ ] **Step 3: Render test** — assert a protected page embeds the token, and an end-to-end FORM-field POST (no header) works:
```rust
#[tokio::test]
async fn form_includes_csrf_and_form_post_succeeds() {
    // logged-in admin server WITHOUT the default header;
    // GET /users, extract value of name="_csrf" from the body,
    // POST /users with (_csrf=<that>, username=.., password=..), assert success (303),
    // and POST without it → 403.
}
```

- [ ] **Step 4: Run**

Run: `cargo nextest run`
Expected: all green.

- [ ] **Step 5: Commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/web.rs templates/*.html tests/*.rs
git commit -m "feat: embed CSRF token as hidden field in browser forms"
```

---

## Task 6: Verification — PostgreSQL parity + full suite + live smoke

- [ ] **Step 1: PG-backed run.** `container ls` → read `pingward-pg` IP → `export TEST_DATABASE_URL=postgres://postgres:postgres@<ip>:5432/postgres` → `cargo nextest run` (0007 applies live; F4 ordering correct on PG). Expected: all green.

- [ ] **Step 2: Live smoke.** Run `./target/debug/pingward` on a throwaway DB + alt port. Confirm: (a) a browser form POST works end-to-end (the rendered form carries `_csrf`); (b) a POST with a wrong/absent `_csrf` and no header → 403; (c) `POST /ping/{uuid}` still succeeds with no token. Show the results.

- [ ] **Step 3: Full suite once more + clippy.** `cargo nextest run` (green) and `cargo clippy --all-targets -- -D warnings` (clean). Commit any final fixups.

---

## Self-Review Notes
- Spec §1 (F1–F4) → Tasks 1–3. §2 (F5 CSRF) → Tasks 4–5 (foundation+harness, then form embedding). §2 test strategy (header escape hatch) → Task 4 Step 6. §3 sequencing honored (CSRF last). §4 out-of-scope (ping exemption) enforced structurally in Task 4 Step 5 + tested in Task 4 Step 7c.
- Risk: Task 4 (CSRF middleware + harness) is the largest and touches every test file's helper. If body-buffering reconstruction proves fragile, the header path already keeps the suite green; the form-field path is exercised by Task 5 Step 3.
