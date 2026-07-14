# Admin Plan 2 — User Management Enhancements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let admins reset another user's password, toggle a user's admin flag, and disable/enable accounts — all audited and guarded against lockout — plus audit the existing create/delete actions.

**Architecture:** Extends the existing `/users` admin surface. New store mutators, three new POST routes, and per-row controls in `users.html`. Every management action writes an `audit_log` entry via the foundation from Plan 1. Guards prevent removing/disabling the last admin and self-lockout.

**Tech Stack:** Rust, axum 0.8, sqlx 0.9 `Any`, askama, argon2, chrono. Tests via `cargo nextest run` with `axum-test`.

## Global Constraints

- **Depends on Plan 1** (`audit_log`, `NewAudit`, `Store::record_audit`, `Store::set_user_disabled`, `User.disabled`). Do not start until Plan 1 is merged.
- Boolean-ish columns read as `!= 0`. sqlx `Any`: `$N` placeholders, `RETURNING id`. Timestamps RFC3339 TEXT.
- `cargo nextest run` (not `cargo test`); `cargo fmt` before every commit; GPG-signed commits; stage files by name.
- Preserve the existing lockout guard: never delete/demote/disable the last remaining admin, and an admin may not disable or delete themselves.
- **Test-helper facts:** the `src/store.rs` inline test store builder is **`seeded()`** (substitute for `test_store()` below); it pre-seeds user `'u'` (id 1) and project `'p'` — avoid reusing username `'u'` and account for the seed in counts. `tests/users_admin.rs` should copy the `admin_server()` helper shape from `tests/admin.rs` (Plan 1).

---

## File Structure

- `src/store.rs` — add `set_user_password`, `set_user_admin`; helper `count_enabled_admins()`.
- `src/web.rs` — add `users_set_password`, `users_toggle_admin`, `users_set_disabled` handlers + routes; add audit writes to `users_create` and `users_delete`; extend guards.
- `templates/users.html` — per-row reset-password form, promote/demote button, disable/enable button, disabled pill.
- `tests/users_admin.rs` — new integration tests.

---

## Task 1: `set_user_password` + `set_user_admin` + `count_enabled_admins`

**Files:**
- Modify: `src/store.rs`
- Test: `src/store.rs` inline tests

**Interfaces:**
- Produces:
  - `Store::set_user_password(&self, id: i64, password_hash: &str) -> Result<(), sqlx::Error>`
  - `Store::set_user_admin(&self, id: i64, is_admin: bool) -> Result<(), sqlx::Error>`
  - `Store::count_enabled_admins(&self) -> Result<i64, sqlx::Error>` (admins with `disabled = 0`)

- [ ] **Step 1: Write failing tests**

```rust
    #[tokio::test]
    async fn set_password_then_login_hash_changes() {
        let store = test_store().await;
        let id = store.create_user("u", Some("old"), false, Utc::now()).await.unwrap();
        store.set_user_password(id, "newphc").await.unwrap();
        let u = store.find_user_by_id(id).await.unwrap().unwrap();
        assert_eq!(u.password_hash.as_deref(), Some("newphc"));
    }

    #[tokio::test]
    async fn set_admin_and_count_enabled_admins() {
        let store = test_store().await;
        let a = store.create_user("a", Some("p"), true, Utc::now()).await.unwrap();
        let b = store.create_user("b", Some("p"), false, Utc::now()).await.unwrap();
        assert_eq!(store.count_enabled_admins().await.unwrap(), 1);
        store.set_user_admin(b, true).await.unwrap();
        assert_eq!(store.count_enabled_admins().await.unwrap(), 2);
        store.set_user_disabled(a, true).await.unwrap();
        assert_eq!(store.count_enabled_admins().await.unwrap(), 1);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo nextest run -p pingward store::tests::set_password_then_login_hash_changes store::tests::set_admin_and_count_enabled_admins`
Expected: FAIL (methods missing).

- [ ] **Step 3: Implement**

```rust
    pub async fn set_user_password(&self, id: i64, password_hash: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
            .bind(password_hash)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_user_admin(&self, id: i64, is_admin: bool) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE users SET is_admin = $1 WHERE id = $2")
            .bind(is_admin as i64)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn count_enabled_admins(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE is_admin <> 0 AND disabled = 0")
            .fetch_one(&self.pool)
            .await
    }
```

- [ ] **Step 4: Run and verify pass**

Run: `cargo nextest run -p pingward store::tests::set_password_then_login_hash_changes store::tests::set_admin_and_count_enabled_admins`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add src/store.rs
git commit -m "feat: add set_user_password/set_user_admin/count_enabled_admins store methods"
```

---

## Task 2: Audit existing create/delete + a shared user-audit helper

**Files:**
- Modify: `src/web.rs` (`users_create`, `users_delete`)
- Test: `tests/users_admin.rs` (created here)

**Interfaces:**
- Produces: audit rows `action = "user.create"` / `"user.delete"` (`target_type = "user"`, `target_id = <affected user id>`, `actor_* = the admin`).

- [ ] **Step 1: Write failing tests**

Create `tests/users_admin.rs` with a logged-in-admin helper (copy the `admin_server()` shape from `tests/admin.rs`), then:
```rust
#[tokio::test]
async fn creating_user_is_audited() {
    let (server, store, _admin) = admin_server().await;
    server
        .post("/users")
        .form(&[("username", "carol"), ("password", "pw123456")])
        .await;
    let carol = store.find_user_by_username("carol").await.unwrap().unwrap();
    let audit = store.list_audit(50).await.unwrap();
    assert!(audit.iter().any(|a| a.action == "user.create"
        && a.target_type.as_deref() == Some("user")
        && a.target_id == Some(carol.id)));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo nextest run --test users_admin creating_user_is_audited`
Expected: FAIL (no audit row).

- [ ] **Step 3: Add audit to `users_create`**

In `src/web.rs` `users_create`, change the extractor from `_admin: AdminUser` to `AdminUser(admin): AdminUser`, and after `create_user(...)` returns the new id, record:
```rust
    let new_id = state
        .store
        .create_user(form.username.trim(), Some(&phc), is_admin, Utc::now())
        .await?;
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "user.create",
                target_type: Some("user"),
                target_id: Some(new_id),
                detail: Some(if is_admin { "admin" } else { "member" }),
                ..Default::default()
            },
            Utc::now(),
        )
        .await?;
```
(`create_user` already returns the id.)

- [ ] **Step 4: Add audit to `users_delete`**

In `users_delete`, after the guards pass and `delete_user(id)` succeeds, record `action = "user.delete"`, `target_id = Some(id)`, using the same `NewAudit { .. ..Default::default() }` shape with `actor_* = admin`.

- [ ] **Step 5: Run and verify pass**

Run: `cargo nextest run --test users_admin creating_user_is_audited`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/web.rs tests/users_admin.rs
git commit -m "feat: audit user create/delete admin actions"
```

---

## Task 3: Reset-password route

**Files:**
- Modify: `src/web.rs` (handler + route), `templates/users.html`
- Test: `tests/users_admin.rs`

**Interfaces:**
- Consumes: `hash_password`, `Store::set_user_password`, `record_audit`.
- Produces: `POST /users/{id}/password` (form field `password`), audited `user.password_reset`.

- [ ] **Step 1: Write failing test**

```rust
#[tokio::test]
async fn admin_resets_password_and_target_can_login() {
    let (server, store, _admin) = admin_server().await;
    let phc = pingward::auth::hash_password("original").unwrap();
    store.create_user("dave", Some(&phc), false, chrono::Utc::now()).await.unwrap();
    let dave = store.find_user_by_username("dave").await.unwrap().unwrap();
    server
        .post(&format!("/users/{}/password", dave.id))
        .form(&[("password", "brandnew1")])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
    let updated = store.find_user_by_id(dave.id).await.unwrap().unwrap();
    assert!(pingward::auth::verify_password("brandnew1", updated.password_hash.as_deref().unwrap()));
    assert!(store.list_audit(50).await.unwrap().iter().any(|a| a.action == "user.password_reset"
        && a.target_id == Some(dave.id)));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo nextest run --test users_admin admin_resets_password_and_target_can_login`
Expected: FAIL (route missing → 404/405).

- [ ] **Step 3: Implement handler + route**

```rust
#[derive(Deserialize)]
struct PasswordForm {
    password: String,
}

async fn users_set_password(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
    Form(form): Form<PasswordForm>,
) -> Result<Response, AppError> {
    if form.password.is_empty() {
        return Ok(Redirect::to("/users").into_response());
    }
    let phc = hash_password(&form.password).map_err(|e| AppError::Other(e.to_string().into()))?;
    state.store.set_user_password(id, &phc).await?;
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "user.password_reset",
                target_type: Some("user"),
                target_id: Some(id),
                ..Default::default()
            },
            Utc::now(),
        )
        .await?;
    Ok(Redirect::to("/users").into_response())
}
```
Add route: `.route("/users/{id}/password", post(users_set_password))`.

- [ ] **Step 4: Add the per-row form in `users.html`**

In each user row, add a small inline form:
```html
<form class="inline" method="post" action="/users/{{ u.id }}/password">
  <input type="password" name="password" placeholder="new password" required>
  <button class="btn" type="submit">reset</button>
</form>
```

- [ ] **Step 5: Run and verify pass**

Run: `cargo nextest run --test users_admin admin_resets_password_and_target_can_login`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/web.rs templates/users.html tests/users_admin.rs
git commit -m "feat: admin reset-password route with audit"
```

---

## Task 4: Toggle-admin route (with last-admin guard)

**Files:**
- Modify: `src/web.rs` (handler + route), `templates/users.html`
- Test: `tests/users_admin.rs`

**Interfaces:**
- Consumes: `Store::{find_user_by_id, set_user_admin, count_enabled_admins}`, `record_audit`.
- Produces: `POST /users/{id}/admin`, audited `user.set_admin`. Demoting the last enabled admin is a no-op redirect.

- [ ] **Step 1: Write failing tests**

```rust
#[tokio::test]
async fn promote_and_demote_admin() {
    let (server, store, _admin) = admin_server().await;
    let uid = store.create_user("erin", Some("p"), false, chrono::Utc::now()).await.unwrap();
    // promote
    server.post(&format!("/users/{uid}/admin")).await.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(uid).await.unwrap().unwrap().is_admin);
    // demote back
    server.post(&format!("/users/{uid}/admin")).await;
    assert!(!store.find_user_by_id(uid).await.unwrap().unwrap().is_admin);
    assert!(store.list_audit(50).await.unwrap().iter().any(|a| a.action == "user.set_admin"));
}

#[tokio::test]
async fn cannot_demote_last_admin() {
    let (server, store, admin_id) = admin_server().await;
    // The only admin demoting themselves is refused.
    server.post(&format!("/users/{admin_id}/admin")).await;
    assert!(store.find_user_by_id(admin_id).await.unwrap().unwrap().is_admin);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo nextest run --test users_admin promote_and_demote_admin cannot_demote_last_admin`
Expected: FAIL (route missing).

- [ ] **Step 3: Implement handler + route**

```rust
async fn users_toggle_admin(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/users").into_response());
    };
    let new_admin = !target.is_admin;
    // Refuse to remove the last enabled admin.
    if !new_admin && target.is_admin && !target.disabled {
        if state.store.count_enabled_admins().await? <= 1 {
            return Ok(Redirect::to("/users").into_response());
        }
    }
    state.store.set_user_admin(id, new_admin).await?;
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "user.set_admin",
                target_type: Some("user"),
                target_id: Some(id),
                detail: Some(if new_admin { "promote" } else { "demote" }),
                ..Default::default()
            },
            Utc::now(),
        )
        .await?;
    Ok(Redirect::to("/users").into_response())
}
```
Add route: `.route("/users/{id}/admin", post(users_toggle_admin))`.

- [ ] **Step 4: Add the per-row control in `users.html`**

```html
<form class="inline" method="post" action="/users/{{ u.id }}/admin">
  <button class="btn" type="submit">{% if u.is_admin %}revoke admin{% else %}make admin{% endif %}</button>
</form>
```

- [ ] **Step 5: Run and verify pass**

Run: `cargo nextest run --test users_admin promote_and_demote_admin cannot_demote_last_admin`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/web.rs templates/users.html tests/users_admin.rs
git commit -m "feat: toggle-admin route with last-admin guard and audit"
```

---

## Task 5: Disable/enable route (with self + last-admin guards)

**Files:**
- Modify: `src/web.rs` (handler + route), `templates/users.html`
- Test: `tests/users_admin.rs`

**Interfaces:**
- Consumes: `Store::{find_user_by_id, set_user_disabled, count_enabled_admins}`, `record_audit`.
- Produces: `POST /users/{id}/disabled`, audited `user.set_disabled`. Guards: cannot disable yourself; cannot disable the last enabled admin.

- [ ] **Step 1: Write failing tests**

```rust
#[tokio::test]
async fn disable_and_enable_member() {
    let (server, store, _admin) = admin_server().await;
    let uid = store.create_user("frank", Some("p"), false, chrono::Utc::now()).await.unwrap();
    server.post(&format!("/users/{uid}/disabled")).await.assert_status(axum::http::StatusCode::SEE_OTHER);
    assert!(store.find_user_by_id(uid).await.unwrap().unwrap().disabled);
    server.post(&format!("/users/{uid}/disabled")).await;
    assert!(!store.find_user_by_id(uid).await.unwrap().unwrap().disabled);
    assert!(store.list_audit(50).await.unwrap().iter().any(|a| a.action == "user.set_disabled"));
}

#[tokio::test]
async fn cannot_disable_self_or_last_admin() {
    let (server, store, admin_id) = admin_server().await;
    server.post(&format!("/users/{admin_id}/disabled")).await;
    assert!(!store.find_user_by_id(admin_id).await.unwrap().unwrap().disabled);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo nextest run --test users_admin disable_and_enable_member cannot_disable_self_or_last_admin`
Expected: FAIL (route missing).

- [ ] **Step 3: Implement handler + route**

```rust
async fn users_set_disabled(
    State(state): State<AppState>,
    AdminUser(admin): AdminUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    // Never disable yourself.
    if id == admin.id {
        return Ok(Redirect::to("/users").into_response());
    }
    let Some(target) = state.store.find_user_by_id(id).await? else {
        return Ok(Redirect::to("/users").into_response());
    };
    let new_disabled = !target.disabled;
    // Refuse to disable the last enabled admin.
    if new_disabled && target.is_admin && !target.disabled {
        if state.store.count_enabled_admins().await? <= 1 {
            return Ok(Redirect::to("/users").into_response());
        }
    }
    state.store.set_user_disabled(id, new_disabled).await?;
    state
        .store
        .record_audit(
            &crate::store::NewAudit {
                actor_user_id: admin.id,
                actor_username: &admin.username,
                action: "user.set_disabled",
                target_type: Some("user"),
                target_id: Some(id),
                detail: Some(if new_disabled { "disable" } else { "enable" }),
                ..Default::default()
            },
            Utc::now(),
        )
        .await?;
    Ok(Redirect::to("/users").into_response())
}
```
Add route: `.route("/users/{id}/disabled", post(users_set_disabled))`.

- [ ] **Step 4: Add the per-row control + disabled pill in `users.html`**

Status cell — show a `disabled` pill when applicable:
```html
{% if u.disabled %}<span class="pill down">disabled</span>{% endif %}
```
Action:
```html
<form class="inline" method="post" action="/users/{{ u.id }}/disabled">
  <button class="btn" type="submit">{% if u.disabled %}enable{% else %}disable{% endif %}</button>
</form>
```

- [ ] **Step 5: Run and verify pass**

Run: `cargo nextest run --test users_admin disable_and_enable_member cannot_disable_self_or_last_admin`
Expected: PASS. Then `cargo nextest run` — full suite green.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add src/web.rs templates/users.html tests/users_admin.rs
git commit -m "feat: disable/enable account route with self and last-admin guards"
```

---

## Task 6: PostgreSQL parity + full suite

- [ ] **Step 1: Run PG-backed tests**

Run: `cargo nextest run --test pg_store` (start Postgres via the `container` CLI per the local-Postgres memory if needed).
Expected: PASS.

- [ ] **Step 2: Full suite**

Run: `cargo nextest run`
Expected: all green.

---

## Self-Review Notes

- Spec §2 (#2 capabilities: password reset, admin toggle, disable/enable) → Tasks 3–5. §4 store methods → Task 1 (`set_user_disabled` already landed in Plan 1). Audit for all management actions incl. create/delete → Task 2 + per-handler audit in 3–5. Guards (self, last-admin) → Tasks 4–5.
- No username rename (explicitly out of scope). `count_enabled_admins` counts only `is_admin AND NOT disabled`, so a disabled admin does not "hold the seat" — consistent with disable/demote guards.
