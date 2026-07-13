# Send Test Notification + Enriched Notification History — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a per-channel "Send test" button on the project page, and add Channel + Error columns to the check page's notification history.

**Architecture:** New `EventKind::Test` gives test sends unambiguous wording. A `POST /channels/{id}/test` handler builds a synthetic event, sends once (no retry) via the existing `notifier_for`, and re-renders the project page with a result banner — no DB write. The check page's existing notification list is enriched by joining channel names in-memory and surfacing the stored error text.

**Tech Stack:** Rust, axum 0.8, sqlx 0.9 (Any), askama 0.16, reqwest; tests via `cargo nextest`, `axum-test` 21 + `wiremock` 0.6.

## Global Constraints

- Tests run with `cargo nextest run` (never `cargo test`).
- `cargo fmt` before every commit; `cargo clippy --all-targets -- -D warnings` must pass.
- All commits GPG-signed; end message with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- No database schema change; no new dependency.
- Test sends MUST NOT call `record_notification` (not persisted to history).
- `check_id: 0` sentinel is used for the synthetic test event (no real check).

---

### Task 1: Add `EventKind::Test`

**Files:**
- Modify: `src/notify.rs` (EventKind enum, `as_str`, `FromStr`, `event_text`, ntfy + pushover match arms)
- Test: `src/notify.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing new.
- Produces: `EventKind::Test` variant; `event_text` renders a test sentence; every `match ev.event` in the file stays exhaustive.

- [ ] **Step 1: Write the failing test**

Add to `src/notify.rs` tests module (after `reminder_event_roundtrips_and_renders_still_down`):

```rust
    #[test]
    fn test_event_roundtrips_and_renders() {
        assert_eq!(EventKind::Test.as_str(), "test");
        assert_eq!(std::str::FromStr::from_str("test"), Ok(EventKind::Test));
        let ev = NotificationEvent {
            check_id: 0,
            check_name: "my-slack".into(),
            event: EventKind::Test,
            at: Utc::now(),
            project_id: 1,
        };
        let text = event_text(&ev);
        assert!(text.contains("test notification"), "got: {text}");
        assert!(text.contains("my-slack"), "got: {text}");
        assert_eq!(event_title(&ev), "pingward: my-slack test");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --lib notify::tests::test_event_roundtrips_and_renders`
Expected: FAIL to compile — `EventKind` has no variant `Test`.

- [ ] **Step 3: Add the `Test` variant and its string mappings**

In `src/notify.rs`, extend the enum (lines 5-10):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Down,
    Up,
    Reminder,
    Test,
}
```

In `as_str` (the match at lines 13-19) add:

```rust
            EventKind::Test => "test",
```

In `FromStr` (the match at lines 25-30) add before the `other` arm:

```rust
            "test" => Ok(EventKind::Test),
```

- [ ] **Step 4: Rewrite `event_text` as one exhaustive match**

Replace the whole body of `event_text` (lines 66-77) with:

```rust
fn event_text(ev: &NotificationEvent) -> String {
    let at = ev.at.to_rfc3339();
    let name = &ev.check_name;
    match ev.event {
        EventKind::Test => {
            format!("\u{1F514} pingward test notification for \"{name}\" (as of {at})")
        }
        EventKind::Down => format!("\u{1F534} {name} is DOWN (as of {at})"),
        EventKind::Up => format!("\u{1F7E2} {name} is UP (as of {at})"),
        EventKind::Reminder => format!("\u{1F534} {name} is STILL DOWN (as of {at})"),
    }
}
```

- [ ] **Step 5: Add `Test` arms to the ntfy and pushover priority matches**

In `NtfyNotifier::send`, the `(priority, tags)` match (lines 253-257) add:

```rust
                EventKind::Test => ("default", "bell"),
```

In `PushoverNotifier::send`, the `priority` match (lines 311-315) add:

```rust
                EventKind::Test => "0",
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo nextest run --lib notify`
Expected: PASS (new test green, all existing notify tests green).

- [ ] **Step 7: fmt + clippy**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: no diff from fmt beyond this task; clippy clean (no non-exhaustive-match errors).

- [ ] **Step 8: Commit**

```bash
git add src/notify.rs
git commit -m "feat: add EventKind::Test for test notifications

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Send-test route, handler, and project-page banner

**Files:**
- Modify: `src/web.rs` (import; route; `TestResult`; `ProjectTemplate` field; `render_project_page` helper; `project_show`; `channel_test`)
- Modify: `templates/project.html` (banner + per-channel Send test button)
- Test: `tests/auth_web.rs` (success + failure integration tests)

**Interfaces:**
- Consumes: `EventKind::Test` (Task 1); existing `notify::{notifier_for, NotificationEvent}`, `store::find_channel`, `owned_project`.
- Produces: `POST /channels/{id}/test`; `render_project_page(store, project, test_result) -> Result<Response, AppError>`.

- [ ] **Step 1: Write the failing integration tests**

Add to `tests/auth_web.rs` (end of file):

```rust
#[tokio::test]
async fn send_test_notification_reports_success() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&mock)
        .await;

    let (server, store, pid) = server_with_project().await;
    let chid = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook",
            &format!("{{\"url\":\"{}\"}}", mock.uri()),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let res = server.post(&format!("/channels/{chid}/test")).await;
    res.assert_status_ok();
    assert!(
        res.text().contains("Test notification sent"),
        "got: {}",
        res.text()
    );
}

#[tokio::test]
async fn send_test_notification_reports_failure() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&mock)
        .await;

    let (server, store, pid) = server_with_project().await;
    let chid = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook",
            &format!("{{\"url\":\"{}\"}}", mock.uri()),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let res = server.post(&format!("/channels/{chid}/test")).await;
    res.assert_status_ok();
    assert!(
        res.text().contains("Test notification failed"),
        "got: {}",
        res.text()
    );
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --test auth_web send_test_notification`
Expected: FAIL — route `/channels/{id}/test` not found (404, assertion on 200 fails).

- [ ] **Step 3: Add the notify import and the route**

In `src/web.rs`, add after the existing `use crate::state::AppState;` (line 9):

```rust
use crate::notify::{notifier_for, EventKind, NotificationEvent};
```

In `routes()` (after line 50, the `channel_delete` route), add:

```rust
        .route("/channels/{id}/test", post(channel_test))
```

- [ ] **Step 4: Add `TestResult` and the `ProjectTemplate` field**

Replace the `ProjectTemplate` struct (lines 220-227) with:

```rust
#[derive(Template)]
#[template(path = "project.html")]
struct ProjectTemplate {
    show_nav: bool,
    project: Project,
    checks: Vec<Check>,
    channels: Vec<Channel>,
    test_result: Option<TestResult>,
}

struct TestResult {
    ok: bool,
    message: String,
}
```

- [ ] **Step 5: Add the `render_project_page` helper and simplify `project_show`**

Replace `project_show` (lines 285-300) with:

```rust
/// Render the project page, optionally with a channel-test result banner.
async fn render_project_page(
    store: &Store,
    project: Project,
    test_result: Option<TestResult>,
) -> Result<Response, AppError> {
    let checks = store.list_checks_for_project(project.id).await?;
    let channels = store.list_channels_for_project(project.id).await?;
    Ok(render(&ProjectTemplate {
        show_nav: true,
        project,
        checks,
        channels,
        test_result,
    })?
    .into_response())
}

async fn project_show(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let project = owned_project(&state.store, id, user.id).await?;
    render_project_page(&state.store, project, None).await
}
```

- [ ] **Step 6: Add the `channel_test` handler**

In `src/web.rs`, add immediately after `channel_delete` (after line 873):

```rust
/// Send a one-off test notification to a single channel and re-render the
/// project page with a result banner. Sends once (no retry) and does not
/// record the attempt in the notification history.
async fn channel_test(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<i64>,
) -> Result<Response, AppError> {
    let channel = state
        .store
        .find_channel(id)
        .await?
        .ok_or(AppError::NotFound)?;
    let project = owned_project(&state.store, channel.project_id, user.id).await?;
    let ev = NotificationEvent {
        check_id: 0,
        check_name: channel.name.clone(),
        event: EventKind::Test,
        at: Utc::now(),
        project_id: channel.project_id,
    };
    let result = match notifier_for(&channel) {
        None => TestResult {
            ok: false,
            message: "channel configuration is incomplete".into(),
        },
        Some(n) => match n.send(&ev).await {
            Ok(()) => TestResult {
                ok: true,
                message: format!("Test notification sent to \"{}\"", channel.name),
            },
            Err(e) => TestResult {
                ok: false,
                message: format!("Test notification failed: {e}"),
            },
        },
    };
    render_project_page(&state.store, project, Some(result)).await
}
```

- [ ] **Step 7: Add the banner and Send test button to `project.html`**

In `templates/project.html`, insert after `{% block body %}` (line 2), before the `<h1>`:

```html
{% if let Some(tr) = test_result %}
<p class="{% if tr.ok %}status-up{% else %}status-down{% endif %}">{{ tr.message }}</p>
{% endif %}
```

Replace the channels table row (lines 24-27) with:

```html
{% for ch in channels %}
<tr><td>{{ ch.name }}</td><td>{{ ch.kind.as_str() }}</td>
  <td>
    <form class="inline" method="post" action="/channels/{{ ch.id }}/test"><button>Send test</button></form>
    <form class="inline" method="post" action="/channels/{{ ch.id }}/delete"><button>delete</button></form>
  </td></tr>
{% endfor %}
```

- [ ] **Step 8: Run tests to verify they pass**

Run: `cargo nextest run --test auth_web send_test_notification`
Expected: PASS — both success and failure banners render at 200.

- [ ] **Step 9: fmt + clippy + full suite**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: fmt/clippy clean; whole SQLite suite green.

- [ ] **Step 10: Commit**

```bash
git add src/web.rs templates/project.html tests/auth_web.rs
git commit -m "feat: add per-channel send-test button on the project page

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Enriched notification history (Channel + Error columns)

**Files:**
- Modify: `src/web.rs` (`NotificationRow`; `CheckTemplate.notifications` type; `check_show` mapping; remove now-unused `Notification` import)
- Modify: `templates/check.html` (table header + rows)
- Test: `tests/auth_web.rs` (history renders channel name + error)

**Interfaces:**
- Consumes: existing `store::{list_channels_for_project, bound_channel_ids, list_recent_notifications}`.
- Produces: `NotificationRow { created_at, event, status, channel, error }` rendered on the check page.

- [ ] **Step 1: Write the failing integration test**

Add to `tests/auth_web.rs` (end of file):

```rust
#[tokio::test]
async fn check_page_shows_notification_channel_and_error() {
    let (server, store, pid) = server_with_project().await;
    let cid = store
        .create_check(
            pid,
            "job",
            "cu",
            pingward::models::ScheduleKind::Period,
            Some(60),
            30,
            None,
            "UTC",
        )
        .await
        .unwrap();
    let chid = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "my-hook",
            "{\"url\":\"http://x\"}",
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    store
        .record_notification(
            cid,
            chid,
            pingward::notify::EventKind::Down,
            pingward::models::NotifyStatus::Error,
            Some("status 500"),
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let res = server.get(&format!("/checks/{cid}")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("my-hook"), "channel name missing: {body}");
    assert!(body.contains("status 500"), "error text missing: {body}");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run --test auth_web check_page_shows_notification_channel_and_error`
Expected: FAIL — body lacks `my-hook` / `status 500` (current table shows only When/Event/Status).

- [ ] **Step 3: Add `NotificationRow` and change the template field type**

In `src/web.rs`, add near the other row structs (after `PingRow`/`PingKindWrap`, around line 378):

```rust
struct NotificationRow {
    created_at: String,
    event: &'static str,
    status: &'static str,
    channel: String,
    error: String,
}
```

In `CheckTemplate` (lines 405-414), change the `notifications` field type:

```rust
    notifications: Vec<NotificationRow>,
```

- [ ] **Step 4: Build the channel-name map and map notifications in `check_show`**

In `check_show`, replace the `channel_boxes` block (lines 552-564) with:

```rust
    let bound = state.store.bound_channel_ids(id).await?;
    let project_channels = state
        .store
        .list_channels_for_project(check.project_id)
        .await?;
    let channel_names: std::collections::HashMap<i64, String> =
        project_channels.iter().map(|c| (c.id, c.name.clone())).collect();
    let channel_boxes = project_channels
        .into_iter()
        .map(|c| ChannelBox {
            id: c.id,
            name: c.name,
            kind: c.kind.as_str(),
            bound: bound.contains(&c.id),
        })
        .collect();
```

Replace the `notifications` binding (line 576) with:

```rust
    let notifications = state
        .store
        .list_recent_notifications(id, 20)
        .await?
        .into_iter()
        .map(|n| NotificationRow {
            created_at: n.created_at.to_rfc3339(),
            event: n.event.as_str(),
            status: n.status.as_str(),
            channel: channel_names
                .get(&n.channel_id)
                .cloned()
                .unwrap_or_else(|| "(deleted)".into()),
            error: n.error.unwrap_or_default(),
        })
        .collect();
```

- [ ] **Step 5: Remove the now-unused `Notification` import**

In `src/web.rs`, the `use crate::models::{...}` (lines 6-8) still lists `Notification`, which is no longer referenced. Remove `Notification` from that list so clippy `-D warnings` passes:

```rust
use crate::models::{
    Channel, ChannelKind, Check, CheckStatus, Project, ScheduleKind, User,
};
```

- [ ] **Step 6: Update `check.html` notifications table**

In `templates/check.html`, replace the Recent notifications table (lines 34-37) with:

```html
<h2>Recent notifications</h2>
<table><tr><th>When</th><th>Event</th><th>Status</th><th>Channel</th><th>Error</th></tr>
{% for n in notifications %}<tr><td>{{ n.created_at }}</td><td>{{ n.event }}</td><td>{{ n.status }}</td><td>{{ n.channel }}</td><td>{{ n.error }}</td></tr>{% endfor %}
</table>
```

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo nextest run --test auth_web check_page_shows_notification_channel_and_error`
Expected: PASS — body contains `my-hook` and `status 500`.

- [ ] **Step 8: fmt + clippy + full suite**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: fmt/clippy clean; whole SQLite suite green (63 prior + new tests).

- [ ] **Step 9: Commit**

```bash
git add src/web.rs templates/check.html tests/auth_web.rs
git commit -m "feat: show channel name and error in notification history

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Final Verification (after all tasks)

- [ ] `cargo fmt --all --check` clean.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.
- [ ] `cargo nextest run` — full SQLite suite green.
- [ ] Live PG round-trip: `TEST_DATABASE_URL=postgres://postgres:postgres@192.168.64.3:5432/postgres cargo nextest run --test pg_store` green (no schema/store change, but confirm).

## Self-Review Notes

- **Spec coverage:** EventKind::Test (Task 1), send-test route + banner + no-record (Task 2), Channel+Error columns with `(deleted)` fallback (Task 3) — all spec sections mapped.
- **Deviation from spec testing:** the "assert no notification row recorded" check is dropped — `Store` exposes no count-all method and the test event uses `check_id: 0` (no real check), so a row-count assertion would require new production API for a weak guard. Non-recording is structural: `channel_test` never calls `record_notification`. Success + failure banner assertions cover the observable behavior.
- **Type consistency:** `NotificationRow.event`/`status` are `&'static str` (from `as_str()`); template prints them directly (no `.as_str()` call in `check.html`). `TestResult.ok: bool` drives banner class via `{% if tr.ok %}`.
