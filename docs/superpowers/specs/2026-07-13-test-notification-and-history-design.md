# Send Test Notification + Enriched Notification History — Design

## Goal

Two small, related web features:

1. **Send test notification** — a per-channel "Send test" button on the
   project page that delivers a one-off notification to that channel and
   reports success/failure inline, so a user can verify a channel's
   configuration without waiting for a real Down event.
2. **Enriched notification history** — the check page already lists recent
   notifications (When / Event / Status). Add **Channel** and **Error**
   columns so a failed delivery shows which channel it targeted and why it
   failed.

## Non-Goals (YAGNI)

- Test sends are **not** recorded in the notification history (a project-level
  channel test has no `check_id`; it is pure live feedback).
- No per-check "test all bound channels" button.
- No pagination, filtering, or search on the notification history.

## Architecture

All work lives in the existing web + notify layers. No schema change, no new
dependency.

### Component 1: `EventKind::Test` (`src/notify.rs`)

The test send needs an event kind distinct from the real Down/Up/Reminder
transitions so its wording is unambiguous.

- Add `Test` to the `EventKind` enum.
- `as_str()` → `"test"`; `FromStr` maps `"test"` → `EventKind::Test`.
- `event_text` gains a `Test` arm producing a dedicated sentence rather than
  the `"{name} is {WORD}"` template:

  ```
  🔔 pingward test notification for "<name>" (as of <rfc3339>)
  ```

  where `<name>` is the event's `check_name`. The test handler puts the
  **channel name** in `check_name`, so text-oriented channels (Telegram,
  Slack, ntfy body) name the channel being tested.
- `event_title` (ntfy `Title` header) already derives from `as_str()`, so it
  yields `pingward: <name> test` with no special-casing (the existing
  control-character sanitization still applies).
- Per-notifier priority/tag maps must add a `Test` arm:
  - ntfy: `Test => ("default", "bell")`
  - pushover: `Test => "0"`

### Component 2: Send test route (`src/web.rs`, `templates/project.html`)

- **Route:** `POST /channels/{id}/test`, handler `channel_test`.
- **Ownership:** load the channel via `find_channel`, then
  `owned_project(channel.project_id, user.id)` — same guard as
  `channel_delete`. A channel that does not exist or belongs to another user
  yields `AppError::NotFound`.
- **Delivery:** build a synthetic event and send once, no retry (a test wants
  immediate feedback):

  ```rust
  let ev = NotificationEvent {
      check_id: 0,
      check_name: channel.name.clone(),
      event: EventKind::Test,
      at: Utc::now(),
      project_id: channel.project_id,
  };
  match notifier_for(&channel) {
      None => /* config incomplete */,
      Some(n) => n.send(&ev).await, // Ok | Err(NotifyError)
  }
  ```

- **Result reporting:** re-render the **project page** with a banner. Add a
  field to `ProjectTemplate`:

  ```rust
  struct TestResult { ok: bool, message: String }
  // ProjectTemplate gains: test_result: Option<TestResult>
  ```

  Outcomes → `message`:
  - `notifier_for` returned `None` → `ok: false`,
    `"channel configuration is incomplete"`.
  - `send` Ok → `ok: true`, `"Test notification sent to \"<name>\""`.
  - `send` Err(e) → `ok: false`, `"Test notification failed: <e>"`.

  `project_show` passes `test_result: None`. To avoid duplicating the
  project-page data load, extract a helper that builds a `ProjectTemplate`
  from a loaded project + `test_result`, used by both `project_show` and
  `channel_test`.
- **Not recorded:** `channel_test` never calls `record_notification`.
- **Template (`project.html`):**
  - At the top of `{% block body %}`, when `test_result` is `Some`, render a
    banner styled by `ok` (e.g. `class="status-up"` / `class="status-down"`
    to reuse existing status colors).
  - In the channels table, add a `Send test` button per row:
    `<form class="inline" method="post" action="/channels/{{ ch.id }}/test"><button>Send test</button></form>`
    alongside the existing delete button.

### Component 3: Enriched history (`src/web.rs`, `templates/check.html`)

- In `check_show`, the project's channels are already fetched to build
  `channel_boxes`. Build a `HashMap<i64, String>` (channel id → name) from the
  same data **before** consuming it into `channel_boxes` (or collect into a
  `Vec` once and derive both), so no extra query is issued.
- Map `Vec<Notification>` → `Vec<NotificationRow>`:

  ```rust
  struct NotificationRow {
      created_at: String,   // rfc3339
      event: &'static str,  // n.event.as_str()
      status: &'static str, // n.status.as_str()
      channel: String,      // names.get(&n.channel_id) or "(deleted)"
      error: String,        // n.error.unwrap_or_default()
  }
  ```

- `CheckTemplate.notifications` type changes from `Vec<Notification>` to
  `Vec<NotificationRow>`.
- **Template (`check.html`):** the Recent notifications table header becomes
  `When | Event | Status | Channel | Error`; each row renders the five fields.

## Data Flow

```
User clicks "Send test" on project page
  → POST /channels/{id}/test
  → owned channel → synthetic Test event → notifier_for → send() (1 attempt)
  → re-render project page with banner (no DB write)

Real notifications (Down/Up/Reminder) recorded by deliver_event as before
  → check page GET → check_show joins channel names in-memory
  → check.html renders When/Event/Status/Channel/Error
```

## Error Handling

- Missing/foreign channel → `AppError::NotFound` (existing pattern).
- Incomplete channel config (`notifier_for` → `None`) → failure banner, not an
  HTTP error.
- Delivery failure (`NotifyError`) → failure banner carrying the error text.
- Deleted channel referenced by an old notification row → `(deleted)` in the
  Channel column (no lookup failure).

## Testing

- **`src/notify.rs` unit tests:**
  - `EventKind::Test` roundtrips: `as_str() == "test"`, `from_str("test")`.
  - `event_text` for `Test` contains `"test notification"` and the channel
    name.
  - `event_title` for a `Test` event yields `pingward: <name> test`.
  - Extend any exhaustive `EventKind` loops to include `Test`.
- **`tests/auth_web.rs` integration tests:**
  - Logged-in user, project + webhook channel pointing at a wiremock server
    returning 200: `POST /channels/{id}/test` → 200 and body contains the
    success banner text; assert no notification row was recorded for any check.
  - Same with the mock returning 500: body contains the failure banner text.
- **Regression:** the existing 63 tests (SQLite) and the live-PG round-trip
  stay green. `cargo fmt` clean, `cargo clippy --all-targets -D warnings`
  clean.

## Files Touched

- `src/notify.rs` — `EventKind::Test`; `event_text`/priority/tag arms; tests.
- `src/web.rs` — `channel_test` handler + route; `ProjectTemplate.test_result`
  + `TestResult`; project-page render helper; `NotificationRow` +
  `check_show` mapping; `CheckTemplate.notifications` type change.
- `templates/project.html` — banner + per-channel Send test button.
- `templates/check.html` — Channel/Error columns.
- `tests/auth_web.rs` — send-test integration tests.
