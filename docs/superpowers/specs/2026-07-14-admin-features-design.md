# Pingward Admin Features — Design

Date: 2026-07-14
Status: Approved (design); implementation pending
Branch: `feat/admin-features`

## 1. Scope

Three related admin capabilities, planned together on a shared foundation and
implemented as three separate implementation plans:

- **#1 Cross-user project access** — an admin can *fully manage* every user's
  projects (view, edit, pause/resume, acknowledge, regenerate, delete, manage
  channels), acting as the owner would. Access happens through a dedicated
  `/admin/*` route group. Every cross-user access writes an audit-log entry.
- **#2 User-management enhancements** — beyond today's list/create/delete:
  reset another user's password, toggle a user's admin flag, and
  disable/enable an account. All of these write audit entries.
- **#3 Admin dashboard** — a standalone `/admin` landing page showing
  site-wide check health, notification health, scheduler/background status,
  and resource-scale figures.

The **audit log** is backend-only in this scope (DB writes, no browsing UI). It
covers all of #1's cross-user accesses and all of #2's management actions. A
read-only viewer is an explicit follow-up (Section 8).

### Non-goals / rejected

- Do **not** add an `|| is_admin` bypass to the existing `owned_project()`
  owner gate. That collapses "admin over-reach" and "owner normal access" into
  one indistinguishable code path with no natural audit point. A dedicated
  `/admin/*` route group makes privileged access a code boundary = an audit
  boundary.
- No audit-log browsing UI, no username rename, no batch operations, no audit
  retention/pruning, no per-action fine-grained permissions in this scope.

## 2. Architecture decision (#1)

**Chosen: dedicated `/admin` route group + shared render/logic helpers, with
audit written at the resolve helper (the choke point).**

- New resolver helpers in `web.rs`, symmetric to the existing `owned_project()`:
  `admin_project(state, id, admin, ctx)`, `admin_check(...)`,
  `admin_channel(...)`. They fetch the resource **without** a `user_id` filter
  and, before returning, write one audit entry (`action = 'admin.access'`,
  with method, path, and target owner). This is the single choke point that
  guarantees audit coverage — the admin analogue of `owned_project()`.
- `owned_project()` and the entire normal per-user flow are left **untouched**,
  preserving the 404 (not 403) semantics that stop non-owners enumerating.

Rejected alternatives:

- **`|| is_admin` inside `owned_project`** — see Non-goals; no audit boundary.
- **Same handlers mounted at `/` and `/admin` via a route-scoped extractor** —
  couples the two paths and blurs the audit boundary; hard to assert "this was
  an admin access" in tests.
- **Audit via middleware over `/admin/*`** — opaque; can't cleanly capture the
  target's owner or action semantics. Writing audit at the resolve helper is
  more accurate and more testable while keeping the same coverage guarantee.

## 3. Data model / migrations

SQLite and PostgreSQL migrations are kept in parity, following the conventions
already established in `0001_init` (SQLite `INTEGER PRIMARY KEY AUTOINCREMENT`,
`created_at TEXT`; PostgreSQL mirrors its own `0001_init` id/timestamp types).
sqlx `Any` driver uses `$N` placeholders and `RETURNING id`.

### `0005_user_disabled`

Add a `disabled` column to `users`:

- SQLite: `disabled INTEGER NOT NULL DEFAULT 0`
- PostgreSQL: `disabled BOOLEAN NOT NULL DEFAULT FALSE`

`User` model gains `disabled: bool` (read as `!= 0` like `is_admin`).

### `0006_audit_log`

```
audit_log(
  id              PK autoincrement,
  actor_user_id   INTEGER,          -- who performed the action
  actor_username  TEXT NOT NULL,    -- snapshot; readable after the user is deleted
  action          TEXT NOT NULL,    -- see action vocabulary below
  target_type     TEXT,             -- 'project' | 'check' | 'channel' | 'user'
  target_id       INTEGER,
  target_owner_id INTEGER,          -- owner of the accessed resource (#1)
  method          TEXT,             -- HTTP method (#1 access)
  path            TEXT,             -- request path
  detail          TEXT,             -- optional freeform summary
  created_at      TEXT NOT NULL
)
CREATE INDEX idx_audit_created ON audit_log(created_at);
```

Action vocabulary:

- `admin.access` — #1 cross-user resource access (read or mutate).
- `user.create`, `user.delete`, `user.password_reset`, `user.set_admin`,
  `user.set_disabled` — #2 management actions.

### Scheduler heartbeat

No new table. The scheduler writes `last_scan_at` and the prune job writes
`last_prune_at` into the existing `settings` table each cycle. The dashboard
reads these and flags staleness.

## 4. Store layer additions (`store.rs`)

- Audit: `record_audit(entry)`, `list_audit(limit)` (the latter for tests and a
  future viewer).
- User management: `set_user_password(id, phc)`, `set_user_admin(id, bool)`,
  `set_user_disabled(id, bool)`.
- Unfiltered getters for the admin resolvers: `get_project(id)`,
  `get_check(id)`, `get_channel(id)` (no `user_id` filter).
- Observability aggregates: `count_checks_by_status()`,
  `list_down_checks_with_owner()`, `notification_counts(since)`,
  `channel_failure_rates(since)`, `recent_failed_notifications(limit)`,
  `count_users()`, `count_projects()`, `count_checks()`,
  `count_pings_since(since)`.

## 5. Auth & access (`auth.rs`)

- **Disabled accounts**: a `User` resolved by `resolve_user()` whose `disabled`
  flag is set is treated as not authenticated — both the session-cookie path
  and the forward-auth path return `None`. This makes disabling take effect
  immediately, invalidating existing sessions. `login_submit` additionally
  rejects a disabled user with a clear message.
- `AdminUser` extractor is unchanged (already gates `is_admin`, 403 otherwise).
- New resolver helpers `admin_project` / `admin_check` / `admin_channel` in
  `web.rs` are the audit choke point (Section 2).

## 6. Routes & handlers (`web.rs`)

**Implementation strategy to avoid duplication:** extract the core of each
existing mutating/show handler into a `*_core(resolved, …)` helper. The owner
route and the admin route become thin wrappers that differ only in how the
resource is resolved — `owned_project()` vs `admin_project()` (which also
writes audit). Handler bodies are not duplicated; only the route table grows.

### #1 — `/admin/*` (guarded by `AdminUser`)

- `GET /admin` → dashboard (#3)
- `GET /admin/projects` → all projects across users (with owner column)
- Mirrored resource actions under the `/admin` prefix:
  - `GET|POST /admin/projects/{id}`, `GET /admin/projects/{id}/edit`,
    `POST /admin/projects/{id}/delete`
  - `GET /admin/projects/{pid}/checks/new`, `POST /admin/projects/{pid}/checks`
  - `GET|POST /admin/checks/{id}`, `GET /admin/checks/{id}/edit`,
    `POST /admin/checks/{id}/{pause,resume,ack,regenerate,delete}`
  - `GET /admin/projects/{pid}/channels/new`,
    `POST /admin/projects/{pid}/channels`,
    `POST /admin/channels/{id}/{delete,test}`,
    `POST /admin/checks/{id}/channels`

### #2 — user management (existing `/users`, all audited)

- `POST /users/{id}/password` → reset password
- `POST /users/{id}/admin` → toggle admin flag (keep the "cannot remove the
  last admin" guard, applied to demotion too)
- `POST /users/{id}/disabled` → disable/enable (guards: cannot disable
  yourself; cannot disable the last enabled admin)

## 7. Navigation & UI (templates)

- `base.html` nav shows an `Admin` link (`/admin`) for admins only. Reuse the
  existing Console theme and components; no new design vocabulary.
- `/admin` dashboard: four section cards. The scheduler card shows "last scan N
  seconds ago" and turns red when stale (e.g. > 2× `scan_interval`).
- `users.html`: each row gains a reset-password mini-form, a promote/demote
  admin control, and a disable/enable button; disabled accounts get a pill.
- New templates: `admin_dashboard.html`, `admin_projects.html`. The admin
  project/check views reuse the existing `project.html` / `check.html` through
  the shared render helper, passing an `is_admin_view` flag so action links
  point at `/admin/*`.

## 8. Testing

- **Authorization**: a non-admin hitting `/admin/*` → 403; an admin accessing
  another user's project → 200 **and** exactly one audit row (`list_audit`
  assertion).
- **Disabled accounts**: after disabling, an existing session is invalidated
  (redirect to `/login`); login is rejected; cannot disable yourself or the
  last enabled admin.
- **#2 actions**: each writes an audit entry; promote/demote guard holds;
  password reset lets the user log in with the new password.
- **#3 aggregates**: dashboard queries return correct figures on both SQLite
  and PostgreSQL (reuse the existing dual-engine test mechanism).
- **Migrations**: `0005` and `0006` apply cleanly on both engines.

## 9. Out of scope / follow-ups

- Audit-log browsing UI (`/admin/audit`).
- Username rename, batch operations, audit retention/pruning, and per-action
  fine-grained permissions.

## 10. Decomposition into implementation plans

The work splits into three plans over a shared base (Sections 3–5):

1. **Foundation + #1** — migrations `0005`/`0006`, audit store methods,
   `disabled` login handling, admin resolvers, `/admin/*` cross-user routes.
2. **#2 user management** — password reset, admin toggle, disable/enable, with
   audit and guards; `users.html` UI.
3. **#3 admin dashboard** — scheduler heartbeat, observability aggregates,
   `/admin` dashboard + `admin_projects.html`, nav link.

Plans 2 and 3 depend on the audit and `/admin` foundation from Plan 1.
