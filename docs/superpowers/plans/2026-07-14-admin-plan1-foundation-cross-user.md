# Admin Plan 1 — Foundation + Cross-User Access Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give admins full, audited management of every user's projects through a dedicated `/admin/*` route group, and make disabled accounts unable to authenticate.

**Architecture:** New `audit_log` and `users.disabled` schema. Admin cross-user access flows through new resolver helpers (`admin_project` / `admin_check` / `admin_channel`) that fetch resources without the `user_id` owner filter and write one `audit_log` row per access — the single choke point guaranteeing audit coverage. The existing `owned_project()` gate and the normal per-user flow are left untouched (404 semantics preserved). Owner and admin handlers share the same core logic/render helpers; only resolution differs.

**Tech Stack:** Rust, axum 0.8, sqlx 0.9 `Any` driver (SQLite + PostgreSQL), askama templates, argon2, chrono. Tests via `cargo nextest run` with `axum-test` integration servers on in-memory SQLite; PostgreSQL parity covered by the existing dual-engine mechanism.

## Global Constraints

- Migrations are kept in **parity** across `migrations/sqlite/` and `migrations/postgres/`. Boolean-ish columns follow the existing `is_admin` convention: SQLite `INTEGER NOT NULL DEFAULT 0`, PostgreSQL `BIGINT NOT NULL DEFAULT 0`, read in Rust as `row.get::<i64,_>(..) != 0`.
- Timestamps are stored as RFC3339 **TEXT** in both engines (`created_at TEXT NOT NULL`), written via `chrono::DateTime<Utc>::to_rfc3339()`.
- sqlx `Any` driver: use `$N` placeholders and `RETURNING id`; never `?`.
- Run tests with `cargo nextest run` (never `cargo test`). Run `cargo fmt` before every commit. All commits GPG-signed (default; do not pass `--no-gpg-sign`).
- Stage files explicitly by name; never `git add -A`/`git add .`.
- `owned_project()` in `src/web.rs` and the normal per-user flow MUST NOT change (keep 404-not-403 semantics so non-owners can't enumerate).

### Test-helper facts (verified against `src/store.rs` / `tests/`)

- The `src/store.rs` inline test module's store builder is **`seeded()`** (not `test_store()` — substitute `seeded()` everywhere below). It pre-seeds one user `'u'` (id 1) and one project `'p'`, so: do not reuse username `'u'`, and account for the pre-existing project/user in any count assertions.
- `Store::create_check` signature is `create_check(project_id: i64, name: &str, ping_uuid: &str, kind: ScheduleKind, period_secs: Option<i64>, grace_secs: i64, cron_expr: Option<&str>, timezone: &str) -> Result<i64, _>` — **no `now` argument**, `kind` is the `ScheduleKind` enum, and a unique `ping_uuid` string is required. Example: `store.create_check(pid, "c", "uuid-c", ScheduleKind::Period, Some(3600), 300, None, "UTC").await`.

---

## File Structure

- `migrations/sqlite/0005_user_disabled.sql`, `migrations/postgres/0005_user_disabled.sql` — add `users.disabled`.
- `migrations/sqlite/0006_audit_log.sql`, `migrations/postgres/0006_audit_log.sql` — create `audit_log`.
- `src/models.rs` — add `disabled: bool` to `User`; add `AuditLog` struct.
- `src/store.rs` — `row_to_user` reads `disabled`; add `record_audit`, `list_audit`, `set_user_disabled`, `row_to_audit`, `NewAudit` input struct.
- `src/auth.rs` — `resolve_user` rejects disabled users.
- `src/web.rs` — `login_submit` rejects disabled users; `admin_project`/`admin_check`/`admin_channel` resolvers; `/admin/*` handlers and routes; `ProjectTemplate`/`CheckTemplate` gain an `admin: bool` flag.
- `templates/project.html`, `templates/check.html` — action URLs respect the `admin` flag.
- `templates/base.html` — `Admin` nav link for admins.
- `templates/admin_projects.html` — all-projects listing (new).
- `tests/admin.rs` — new integration tests for cross-user access + audit.
- `tests/auth_web.rs` — disabled-login tests.

---

## Task 1: `users.disabled` column + model

**Files:**
- Create: `migrations/sqlite/0005_user_disabled.sql`, `migrations/postgres/0005_user_disabled.sql`
- Modify: `src/models.rs` (User struct), `src/store.rs` (`row_to_user`)
- Test: `src/store.rs` inline test module

**Interfaces:**
- Produces: `User.disabled: bool` (read as `!= 0`), populated by `row_to_user`.

- [ ] **Step 1: Write the migrations**

`migrations/sqlite/0005_user_disabled.sql`:
```sql
ALTER TABLE users ADD COLUMN disabled INTEGER NOT NULL DEFAULT 0;
```

`migrations/postgres/0005_user_disabled.sql`:
```sql
ALTER TABLE users ADD COLUMN disabled BIGINT NOT NULL DEFAULT 0;
```

- [ ] **Step 2: Add the model field**

In `src/models.rs`, add to `struct User` after `is_admin`:
```rust
    pub disabled: bool,
```

- [ ] **Step 3: Populate it in `row_to_user`**

In `src/store.rs`, in `row_to_user`, add after the `is_admin` line:
```rust
        disabled: row.get::<i64, _>("disabled") != 0,
```

- [ ] **Step 4: Write a failing test**

Add to the `#[cfg(test)] mod tests` in `src/store.rs` (use the existing test-store helper in that module — mirror a neighbouring test's setup):
```rust
    #[tokio::test]
    async fn new_user_is_not_disabled() {
        let store = test_store().await;
        let id = store
            .create_user("u", Some("phc"), false, Utc::now())
            .await
            .unwrap();
        let u = store.find_user_by_id(id).await.unwrap().unwrap();
        assert!(!u.disabled);
    }
```

- [ ] **Step 5: Run and verify pass**

Run: `cargo nextest run -p pingward store::tests::new_user_is_not_disabled`
Expected: PASS (existing `create_user` unchanged; column defaults to 0).

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add migrations/sqlite/0005_user_disabled.sql migrations/postgres/0005_user_disabled.sql src/models.rs src/store.rs
git commit -m "feat: add users.disabled column and model field"
```

---

## Task 2: `audit_log` table + store writes

**Files:**
- Create: `migrations/sqlite/0006_audit_log.sql`, `migrations/postgres/0006_audit_log.sql`
- Modify: `src/models.rs` (add `AuditLog`), `src/store.rs` (`NewAudit`, `record_audit`, `list_audit`, `row_to_audit`)
- Test: `src/store.rs` inline test module

**Interfaces:**
- Produces:
  - `struct NewAudit<'a> { actor_user_id: i64, actor_username: &'a str, action: &'a str, target_type: Option<&'a str>, target_id: Option<i64>, target_owner_id: Option<i64>, method: Option<&'a str>, path: Option<&'a str>, detail: Option<&'a str> }`
  - `Store::record_audit(&self, e: &NewAudit<'_>, now: DateTime<Utc>) -> Result<i64, sqlx::Error>`
  - `Store::list_audit(&self, limit: i64) -> Result<Vec<AuditLog>, sqlx::Error>`
  - `struct AuditLog { id, actor_user_id: i64, actor_username: String, action: String, target_type: Option<String>, target_id: Option<i64>, target_owner_id: Option<i64>, method: Option<String>, path: Option<String>, detail: Option<String>, created_at: DateTime<Utc> }`

- [ ] **Step 1: Write the migrations**

`migrations/sqlite/0006_audit_log.sql`:
```sql
CREATE TABLE audit_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  actor_user_id INTEGER,
  actor_username TEXT NOT NULL,
  action TEXT NOT NULL,
  target_type TEXT,
  target_id INTEGER,
  target_owner_id INTEGER,
  method TEXT,
  path TEXT,
  detail TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_audit_created ON audit_log(created_at);
```

`migrations/postgres/0006_audit_log.sql`:
```sql
CREATE TABLE audit_log (
  id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  actor_user_id BIGINT,
  actor_username TEXT NOT NULL,
  action TEXT NOT NULL,
  target_type TEXT,
  target_id BIGINT,
  target_owner_id BIGINT,
  method TEXT,
  path TEXT,
  detail TEXT,
  created_at TEXT NOT NULL
);
CREATE INDEX idx_audit_created ON audit_log(created_at);
```

- [ ] **Step 2: Add the model + input structs**

In `src/models.rs`:
```rust
#[derive(Debug, Clone)]
pub struct AuditLog {
    pub id: i64,
    pub actor_user_id: i64,
    pub actor_username: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<i64>,
    pub target_owner_id: Option<i64>,
    pub method: Option<String>,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub created_at: DateTime<Utc>,
}
```

In `src/store.rs` (near the top of the file, after the imports), add the write-input struct:
```rust
#[derive(Debug, Clone, Default)]
pub struct NewAudit<'a> {
    pub actor_user_id: i64,
    pub actor_username: &'a str,
    pub action: &'a str,
    pub target_type: Option<&'a str>,
    pub target_id: Option<i64>,
    pub target_owner_id: Option<i64>,
    pub method: Option<&'a str>,
    pub path: Option<&'a str>,
    pub detail: Option<&'a str>,
}
```

- [ ] **Step 3: Add `row_to_audit`**

In `src/store.rs`, next to the other `row_to_*` helpers:
```rust
fn row_to_audit(row: &sqlx::any::AnyRow) -> Result<AuditLog, sqlx::Error> {
    Ok(AuditLog {
        id: row.get("id"),
        actor_user_id: row.get("actor_user_id"),
        actor_username: row.get("actor_username"),
        action: row.get("action"),
        target_type: row.get("target_type"),
        target_id: row.get("target_id"),
        target_owner_id: row.get("target_owner_id"),
        method: row.get("method"),
        path: row.get("path"),
        detail: row.get("detail"),
        created_at: parse_ts(row.get("created_at"))
            .ok_or_else(|| decode_err("audit_log.created_at must be RFC3339"))?,
    })
}
```
Add `AuditLog` to the `use crate::models::{...}` line at the top of `src/store.rs`.

- [ ] **Step 4: Add `record_audit` and `list_audit`**

In `impl Store`:
```rust
    pub async fn record_audit(
        &self,
        e: &NewAudit<'_>,
        now: DateTime<Utc>,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query(
            "INSERT INTO audit_log \
             (actor_user_id, actor_username, action, target_type, target_id, \
              target_owner_id, method, path, detail, created_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id",
        )
        .bind(e.actor_user_id)
        .bind(e.actor_username)
        .bind(e.action)
        .bind(e.target_type)
        .bind(e.target_id)
        .bind(e.target_owner_id)
        .bind(e.method)
        .bind(e.path)
        .bind(e.detail)
        .bind(now.to_rfc3339())
        .fetch_one(&self.pool)
        .await?;
        Ok(row.get::<i64, _>("id"))
    }

    pub async fn list_audit(&self, limit: i64) -> Result<Vec<AuditLog>, sqlx::Error> {
        let rows = sqlx::query("SELECT * FROM audit_log ORDER BY id DESC LIMIT $1")
            .bind(limit)
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(row_to_audit).collect()
    }
```

- [ ] **Step 5: Write a failing test**

In `src/store.rs` tests:
```rust
    #[tokio::test]
    async fn audit_roundtrips() {
        let store = test_store().await;
        let uid = store
            .create_user("adm", Some("phc"), true, Utc::now())
            .await
            .unwrap();
        store
            .record_audit(
                &NewAudit {
                    actor_user_id: uid,
                    actor_username: "adm",
                    action: "admin.access",
                    target_type: Some("project"),
                    target_id: Some(7),
                    target_owner_id: Some(42),
                    method: Some("GET"),
                    path: Some("/admin/projects/7"),
                    detail: None,
                },
                Utc::now(),
            )
            .await
            .unwrap();
        let rows = store.list_audit(10).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].action, "admin.access");
        assert_eq!(rows[0].target_owner_id, Some(42));
        assert_eq!(rows[0].actor_username, "adm");
    }
```

- [ ] **Step 6: Run and verify pass**

Run: `cargo nextest run -p pingward store::tests::audit_roundtrips`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add migrations/sqlite/0006_audit_log.sql migrations/postgres/0006_audit_log.sql src/models.rs src/store.rs
git commit -m "feat: add audit_log table and record/list store methods"
```

---

## Task 3: `set_user_disabled` store method

**Files:**
- Modify: `src/store.rs`
- Test: `src/store.rs` inline test module

**Interfaces:**
- Produces: `Store::set_user_disabled(&self, id: i64, disabled: bool) -> Result<(), sqlx::Error>` (reused by Plan 2's disable/enable route).

- [ ] **Step 1: Write a failing test**

```rust
    #[tokio::test]
    async fn set_user_disabled_toggles() {
        let store = test_store().await;
        let id = store
            .create_user("u", Some("phc"), false, Utc::now())
            .await
            .unwrap();
        store.set_user_disabled(id, true).await.unwrap();
        assert!(store.find_user_by_id(id).await.unwrap().unwrap().disabled);
        store.set_user_disabled(id, false).await.unwrap();
        assert!(!store.find_user_by_id(id).await.unwrap().unwrap().disabled);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p pingward store::tests::set_user_disabled_toggles`
Expected: FAIL (`no method named set_user_disabled`).

- [ ] **Step 3: Implement**

In `impl Store`:
```rust
    pub async fn set_user_disabled(&self, id: i64, disabled: bool) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET disabled = $1 WHERE id = $2")
            .bind(disabled as i64)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
```

- [ ] **Step 4: Run and verify pass**

Run: `cargo nextest run -p pingward store::tests::set_user_disabled_toggles`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/store.rs
git commit -m "feat: add set_user_disabled store method"
```

---

## Task 4: Disabled accounts cannot authenticate

**Files:**
- Modify: `src/auth.rs` (`resolve_user`), `src/web.rs` (`login_submit`)
- Test: `tests/auth_web.rs`

**Interfaces:**
- Consumes: `User.disabled`, `Store::set_user_disabled`.
- Behaviour: any request whose resolved user is `disabled` is treated as unauthenticated; explicit login of a disabled user is rejected.

- [ ] **Step 1: Write the failing tests**

Add to `tests/auth_web.rs`:
```rust
#[tokio::test]
async fn disabling_user_invalidates_session() {
    let (server, store, uid) = logged_in_server().await;
    // Authenticated: dashboard is 200.
    server.get("/").await.assert_status_ok();
    // Disable the account, then the same session must redirect to /login.
    store.set_user_disabled(uid, true).await.unwrap();
    let res = server.get("/projects/new").await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert_eq!(res.header("location"), "/login");
}

#[tokio::test]
async fn disabled_user_cannot_log_in() {
    let (server, store) = server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("bob", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    store.set_user_disabled(uid, true).await.unwrap();
    let res = server
        .post("/login")
        .form(&[("username", "bob"), ("password", "pw")])
        .await;
    // Login page re-renders with an error (200), no session cookie set.
    res.assert_status_ok();
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo nextest run --test auth_web disabling_user_invalidates_session disabled_user_cannot_log_in`
Expected: FAIL (disabled user still authenticates / logs in).

- [ ] **Step 3: Reject disabled users in `resolve_user`**

In `src/auth.rs`, wrap the two return points of `resolve_user`. Change the session branch:
```rust
    if let Some(cookie) = jar.get(SESSION_COOKIE) {
        if let Ok(Some(user)) = state.store.find_session_user(cookie.value(), now).await {
            if !user.disabled {
                return Some(user);
            }
        }
    }
```
And the forward-auth branch's existing-user case:
```rust
            Ok(Some(user)) => {
                if !user.disabled {
                    return Some(user);
                }
            }
```
(The auto-provision `Ok(None)` case creates a fresh non-disabled user, so it is unaffected.)

- [ ] **Step 4: Reject disabled users in `login_submit`**

In `src/web.rs`, in `login_submit`, after the user is fetched and the password verifies but before creating the session, add a disabled check that re-renders the login page with an error. Locate the block that verifies the password and, alongside the existing "invalid credentials" failure, add:
```rust
    if user.disabled {
        return Ok(render(&LoginTemplate {
            show_nav: false,
            error: Some("account is disabled".into()),
        })?
        .into_response());
    }
```
(Place this immediately after the successful password check, before `create_session`.)

- [ ] **Step 5: Run and verify pass**

Run: `cargo nextest run --test auth_web disabling_user_invalidates_session disabled_user_cannot_log_in`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/auth.rs src/web.rs
git commit -m "feat: disabled accounts cannot authenticate or log in"
```

---

## Task 5: Admin resolver helpers (audit choke point)

**Files:**
- Modify: `src/web.rs`
- Test: `tests/admin.rs` (created here)

**Interfaces:**
- Consumes: `Store::{find_project, find_check, find_channel, record_audit}`, `NewAudit`, `AdminUser`.
- Produces (in `src/web.rs`, module-private):
  - `async fn admin_project(state: &AppState, id: i64, admin: &User, method: &str, path: &str) -> Result<Project, AppError>`
  - `async fn admin_check(state: &AppState, id: i64, admin: &User, method: &str, path: &str) -> Result<Check, AppError>`
  - `async fn admin_channel(state: &AppState, id: i64, admin: &User, method: &str, path: &str) -> Result<Channel, AppError>`

- [ ] **Step 1: Implement the resolvers**

In `src/web.rs`, near `owned_project`:
```rust
/// Resolve any project by id (no owner filter) and record an admin-access
/// audit entry. The single choke point for #1 cross-user reads and writes.
async fn admin_project(
    state: &AppState,
    id: i64,
    admin: &User,
    method: &str,
    path: &str,
) -> Result<Project, AppError> {
    let p = state.store.find_project(id).await?.ok_or(AppError::NotFound)?;
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "admin.access",
                target_type: Some("project"),
                target_id: Some(p.id),
                target_owner_id: Some(p.user_id),
                method: Some(method),
                path: Some(path),
                detail: None,
            },
            Utc::now(),
        )
        .await?;
    Ok(p)
}

async fn admin_check(
    state: &AppState,
    id: i64,
    admin: &User,
    method: &str,
    path: &str,
) -> Result<Check, AppError> {
    let c = state.store.find_check(id).await?.ok_or(AppError::NotFound)?;
    let owner = state
        .store
        .find_project(c.project_id)
        .await?
        .map(|p| p.user_id);
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "admin.access",
                target_type: Some("check"),
                target_id: Some(c.id),
                target_owner_id: owner,
                method: Some(method),
                path: Some(path),
                detail: None,
            },
            Utc::now(),
        )
        .await?;
    Ok(c)
}

async fn admin_channel(
    state: &AppState,
    id: i64,
    admin: &User,
    method: &str,
    path: &str,
) -> Result<Channel, AppError> {
    let ch = state.store.find_channel(id).await?.ok_or(AppError::NotFound)?;
    let owner = state
        .store
        .find_project(ch.project_id)
        .await?
        .map(|p| p.user_id);
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "admin.access",
                target_type: Some("channel"),
                target_id: Some(ch.id),
                target_owner_id: owner,
                method: Some(method),
                path: Some(path),
                detail: None,
            },
            Utc::now(),
        )
        .await?;
    Ok(ch)
}
```
Ensure `NewAudit` is reachable (either `use crate::store::NewAudit;` at the top or the fully-qualified path above).

- [ ] **Step 2: No standalone test yet**

These are exercised end-to-end in Task 7. Compile-check only:
Run: `cargo build`
Expected: compiles (helpers are `dead_code` until Task 7 wires routes — that is acceptable within this task; do not add `#[allow]`).

- [ ] **Step 3: Commit**

```bash
cargo fmt
git add src/web.rs
git commit -m "feat: add admin_project/check/channel resolvers that write audit"
```

---

## Task 6: Template `admin` flag for owner-vs-admin action URLs

**Files:**
- Modify: `src/web.rs` (`ProjectTemplate`, `CheckTemplate`, `render_project_page`, and the check-render helper), `templates/project.html`, `templates/check.html`
- Test: existing owner-path tests must still pass (regression guard)

**Interfaces:**
- Produces: `ProjectTemplate.admin: bool` and `CheckTemplate.admin: bool`. When `admin` is true, action `<form>`/link URLs use the `/admin` prefix; the owner path passes `admin: false`.

- [ ] **Step 1: Add the flag to the templates' structs**

In `src/web.rs`, add `admin: bool` to `struct ProjectTemplate` and `struct CheckTemplate`. Set `admin: false` at every existing (owner-path) construction site, including inside `render_project_page` and the check-render helper. Introduce a small helper the templates can use, or compute a `base: &str` string ("" for owner, "/admin" for admin). Recommended: add `admin: bool` and derive URLs in-template.

- [ ] **Step 2: Make `project.html` action URLs admin-aware**

In `templates/project.html`, prefix every project/check/channel action URL with the base. Define it once at the top of the template body:
```
{% let base = if admin { "/admin" } else { "" } %}
```
Then replace occurrences such as `action="/projects/{{ project.id }}"` with `action="{{ base }}/projects/{{ project.id }}"`, `/checks/{{ c.id }}/...` with `{{ base }}/checks/{{ c.id }}/...`, `/projects/{{ project.id }}/checks/new` with `{{ base }}/projects/{{ project.id }}/checks/new`, and the channel URLs likewise.

- [ ] **Step 3: Make `check.html` action URLs admin-aware**

Same pattern in `templates/check.html`: add the `{% let base %}` line and prefix the check's action/edit/pause/resume/ack/regenerate/delete/channels URLs and the back-link to its project with `{{ base }}`.

- [ ] **Step 4: Run the existing suite (owner path unchanged)**

Run: `cargo nextest run`
Expected: PASS (owner path passes `admin: false`, so all URLs are unchanged from today).

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/web.rs templates/project.html templates/check.html
git commit -m "feat: make project/check templates render admin-prefixed action URLs"
```

---

## Task 7: `/admin` route group — cross-user handlers + routes

**Files:**
- Modify: `src/web.rs` (admin handlers + `routes()`), `templates/base.html`
- Create: `templates/admin_projects.html`
- Test: `tests/admin.rs`

**Interfaces:**
- Consumes: `admin_project`/`admin_check`/`admin_channel`, `AdminUser`, existing render helpers (`render_project_page`, the check-render helper) and existing store mutators (`update_project`, `update_check`, `delete_check`, etc.).
- Produces: `/admin`, `/admin/projects`, and the mirrored `/admin/projects/*`, `/admin/checks/*`, `/admin/channels/*` routes.

- [ ] **Step 1: Write the failing cross-user + audit tests**

Create `tests/admin.rs`:
```rust
use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

async fn admin_server() -> (TestServer, Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    let admin_id = store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", "admin"), ("password", "pw")])
        .await;
    (server, store, admin_id)
}

#[tokio::test]
async fn non_admin_forbidden_on_admin_routes() {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    store
        .create_user("member", Some(&phc), false, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", "member"), ("password", "pw")])
        .await;
    server
        .get("/admin")
        .await
        .assert_status(axum::http::StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn admin_views_other_users_project_and_audits() {
    let (server, store, _admin_id) = admin_server().await;
    // A separate user owns a project + check.
    let owner = store
        .create_user("owner", Some("phc"), false, chrono::Utc::now())
        .await
        .unwrap();
    let pid = store
        .create_project(owner, "victim", None, None, chrono::Utc::now())
        .await
        .unwrap();
    // Admin can see it via /admin, owner-scoped route would 404.
    server
        .get("/projects")
        .await; // (owner route is per-user; admin uses /admin)
    server
        .get(&format!("/admin/projects/{pid}"))
        .await
        .assert_status_ok();
    let audit = store.list_audit(10).await.unwrap();
    assert!(audit
        .iter()
        .any(|a| a.action == "admin.access"
            && a.target_type.as_deref() == Some("project")
            && a.target_id == Some(pid)
            && a.target_owner_id == Some(owner)));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo nextest run --test admin`
Expected: FAIL (no `/admin` routes yet → 404/redirect, not 403/200).

- [ ] **Step 3: Add the admin projects-list handler + template**

Create `templates/admin_projects.html` (reuse Console styling from `dashboard.html`; each row links to `/admin/projects/{{ p.id }}` and shows the owner username). Add a store helper `list_all_projects_with_owner()` returning `Vec<(Project, String)>`:
```rust
    pub async fn list_all_projects_with_owner(
        &self,
    ) -> Result<Vec<(Project, String)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT p.*, u.username AS owner_username \
             FROM projects p JOIN users u ON u.id = p.user_id ORDER BY p.id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.iter()
            .map(|r| Ok((row_to_project(r)?, r.get::<String, _>("owner_username"))))
            .collect()
    }
```
Add `AdminProjectsTemplate { show_nav: bool, projects: Vec<(Project, String)> }` and a handler:
```rust
async fn admin_projects_page(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Response, AppError> {
    let projects = state.store.list_all_projects_with_owner().await?;
    Ok(render(&AdminProjectsTemplate { show_nav: true, projects })?.into_response())
}
```
For `/admin` itself in this plan, redirect to `/admin/projects` (Plan 3 replaces it with the dashboard):
```rust
async fn admin_home(_admin: AdminUser) -> Result<Response, AppError> {
    Ok(Redirect::to("/admin/projects").into_response())
}
```

- [ ] **Step 4: Add the mirrored resource handlers**

Each admin handler is a thin wrapper: resolve via `admin_*` (writing audit), then call the same core/render/mutator the owner handler uses, redirecting under `/admin`. Extract `axum::http::Method` and `axum::http::Uri` to feed the resolver. Example for the project show + update + delete:
```rust
async fn admin_project_show(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
    render_project_page_admin(&state.store, project, None).await // renders with admin: true
}

async fn admin_project_update(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
    Form(form): Form<ProjectForm>,
) -> Result<Response, AppError> {
    admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
    state
        .store
        .update_project(id, &form.name, parse_opt_i64(&form.scan_interval_secs), parse_opt_i64(&form.nag_interval_secs))
        .await?;
    Ok(Redirect::to(&format!("/admin/projects/{id}")).into_response())
}

async fn admin_project_delete(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    method: axum::http::Method,
    uri: axum::http::Uri,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    admin_project(&state, id, &admin, method.as_str(), uri.path()).await?;
    state.store.delete_project(id).await?;
    Ok(Redirect::to("/admin/projects").into_response())
}
```
Provide `render_project_page_admin` by generalizing `render_project_page` to take an `admin: bool` (owner call passes `false`, admin passes `true`) rather than duplicating it — add the parameter and thread it into `ProjectTemplate.admin`. Do the same for the check-render helper (`render_check_page(..., admin: bool)`).

Repeat the same thin-wrapper pattern, one wrapper per existing owner action, for:
- `admin_project_edit` → renders `ProjectFormTemplate` with `action: /admin/projects/{id}`.
- `admin_check_new`, `admin_check_create` (resolve the parent project via `admin_project`, then reuse the owner create logic; redirect under `/admin`).
- `admin_check_show`, `admin_check_update`, `admin_check_edit`, `admin_check_pause`, `admin_check_resume`, `admin_check_ack`, `admin_check_regenerate`, `admin_check_delete` (resolve via `admin_check`).
- `admin_channel_new`, `admin_channel_create` (resolve parent project via `admin_project`), `admin_channel_delete`, `admin_channel_test` (resolve via `admin_channel`), `admin_check_set_channels` (resolve via `admin_check`).

Each wrapper body mirrors its owner counterpart's core exactly, differing only in (a) resolving through the `admin_*` helper and (b) redirecting to the `/admin`-prefixed path. Keep the shared work (validation, store mutators, render helpers) factored so the wrapper stays ~4–8 lines.

- [ ] **Step 5: Register the routes**

In `routes()`, append the admin group (guarded per-handler by `AdminUser`):
```rust
        .route("/admin", get(admin_home))
        .route("/admin/projects", get(admin_projects_page))
        .route("/admin/projects/{id}", get(admin_project_show).post(admin_project_update))
        .route("/admin/projects/{id}/edit", get(admin_project_edit))
        .route("/admin/projects/{id}/delete", post(admin_project_delete))
        .route("/admin/projects/{pid}/checks/new", get(admin_check_new))
        .route("/admin/projects/{pid}/checks", post(admin_check_create))
        .route("/admin/checks/{id}", get(admin_check_show).post(admin_check_update))
        .route("/admin/checks/{id}/edit", get(admin_check_edit))
        .route("/admin/checks/{id}/pause", post(admin_check_pause))
        .route("/admin/checks/{id}/resume", post(admin_check_resume))
        .route("/admin/checks/{id}/ack", post(admin_check_ack))
        .route("/admin/checks/{id}/regenerate", post(admin_check_regenerate))
        .route("/admin/checks/{id}/delete", post(admin_check_delete))
        .route("/admin/projects/{pid}/channels/new", get(admin_channel_new))
        .route("/admin/projects/{pid}/channels", post(admin_channel_create))
        .route("/admin/channels/{id}/delete", post(admin_channel_delete))
        .route("/admin/channels/{id}/test", post(admin_channel_test))
        .route("/admin/checks/{id}/channels", post(admin_check_set_channels))
```

- [ ] **Step 6: Add the nav link**

In `templates/base.html`, in the nav, add an `Admin` link for admins. The template needs the current user's admin flag; thread an `is_admin: bool` field through the templates that extend `base.html` (default `false`), and set it `true` where an admin is logged in. Minimal version for this plan: show the link only on admin pages (where the struct sets `is_admin: true`). Render:
```
{% if is_admin %}<a href="/admin">Admin</a>{% endif %}
```

- [ ] **Step 7: Run and verify pass**

Run: `cargo nextest run --test admin`
Expected: PASS. Then run the full suite: `cargo nextest run` — all green.

- [ ] **Step 8: Commit**

```bash
cargo fmt
git add src/web.rs src/store.rs templates/admin_projects.html templates/base.html tests/admin.rs
git commit -m "feat: add /admin cross-user route group with audited full management"
```

---

## Task 8: Mutation-audit coverage test

**Files:**
- Test: `tests/admin.rs`

**Interfaces:**
- Consumes: `/admin/checks/{id}/pause`, `Store::list_audit`, `Store::find_check`.

- [ ] **Step 1: Write the test**

```rust
#[tokio::test]
async fn admin_mutation_on_other_project_is_audited() {
    let (server, store, _admin_id) = admin_server().await;
    let owner = store
        .create_user("owner2", Some("phc"), false, chrono::Utc::now())
        .await
        .unwrap();
    let pid = store
        .create_project(owner, "p", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let cid = store
        .create_check(pid, "c", "uuid-c", pingward::models::ScheduleKind::Period, Some(3600), 300, None, "UTC")
        .await
        .unwrap();
    server
        .post(&format!("/admin/checks/{cid}/pause"))
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    // Check is paused and the access was audited.
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().status,
        pingward::models::CheckStatus::Paused
    );
    let audit = store.list_audit(50).await.unwrap();
    assert!(audit.iter().any(|a| a.target_type.as_deref() == Some("check")
        && a.target_id == Some(cid)
        && a.method.as_deref() == Some("POST")));
}
```
The point of the test is the pause + audit assertion.

- [ ] **Step 2: Run and verify pass**

Run: `cargo nextest run --test admin admin_mutation_on_other_project_is_audited`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/admin.rs
git commit -m "test: assert admin mutations on other users' checks are audited"
```

---

## Task 9: PostgreSQL parity + full-suite green

**Files:**
- Modify: `tests/pg_store.rs` (only if it enumerates migrations or per-table checks)

- [ ] **Step 1: Confirm migrations apply on PostgreSQL**

Run the PG-backed tests per the existing mechanism (see `tests/pg_store.rs` and the local-Postgres memory note — start Postgres via the `container` CLI if needed, export `PINGWARD_TEST_PG_URL` or the variable that file uses):
Run: `cargo nextest run --test pg_store`
Expected: PASS (0005/0006 apply; `audit_log` and `disabled` usable).

- [ ] **Step 2: Full suite**

Run: `cargo nextest run`
Expected: all green.

- [ ] **Step 3: Commit any parity fixes**

```bash
cargo fmt
git add -u tests/pg_store.rs   # only if changed; otherwise skip
git commit -m "test: cover admin foundation migrations on postgres"
```

---

## Self-Review Notes

- Spec §3 (migrations) → Tasks 1–2. §4 (store) → Tasks 2–3, 7 (`list_all_projects_with_owner`). §5 (auth/disabled + resolvers) → Tasks 4–5. §6 (routes/handlers #1) → Task 7. §7 (nav/templates) → Tasks 6–7. §8 (testing) → Tasks 4,7,8,9.
- #2 (user-management routes/UI) and #3 (dashboard, scheduler heartbeat, observability aggregates) are intentionally **out of this plan** — separate plan docs, built on the audit + `/admin` foundation landed here. `set_user_disabled` is added here (Task 3) because disabled-login enforcement needs it; Plan 2 reuses it.
- `/admin` is a redirect to `/admin/projects` in this plan; Plan 3 replaces `admin_home` with the dashboard handler.
