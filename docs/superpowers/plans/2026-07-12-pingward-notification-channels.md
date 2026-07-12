# Notification Channels (Plan 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add Telegram, Slack, ntfy, and Pushover notification channels alongside the existing webhook channel, so a check's down/up events can be delivered to those services.

**Architecture:** Each channel type is a `Notifier` trait impl selected by `notifier_for(&Channel)` from the channel's `(kind, config_json)` row. Plan 2 already wired `deliver_event` → `notifier_for` → `send_with_retry` → `record_notification`; this plan only adds three `Notifier` impls, extends `notifier_for`'s match, and expands the channel-creation UI/handler to capture each kind's config. No scheduling, state-machine, or DB-schema changes.

**Tech Stack:** Rust + tokio, `reqwest` (already a dep, used by `WebhookNotifier`), `serde_json` for config, `askama` templates, `wiremock` for tests.

## Global Constraints

- `sqlx` 0.9, `axum` 0.8, `askama` 0.16 — do not change versions.
- No third-party dependency published less than 7 days ago. This plan requires NO new dependencies (reqwest/serde_json/wiremock are already present) — do not add any.
- Tests run with `cargo nextest run` (NOT `cargo test`). `cargo fmt` before every commit. `cargo clippy --all-targets -- -D warnings` must stay clean (CI-enforced).
- All SQL lives in `src/store.rs` only — but this plan writes no new SQL; `create_channel` already exists and is reused unchanged.
- `ChannelKind` currently defines `Webhook`, `Telegram`, `Slack`, `Ntfy` (see `src/models.rs:25`, a `str_enum!` macro). This plan ADDS a `Pushover => "pushover"` variant (Task 4). Do not otherwise restructure the enum.
- A failing channel must never propagate an error into state (spec §6). `deliver_event` already guarantees this; notifiers only return `Result<(), NotifyError>` and the existing plumbing records failures.
- `config_json` is the single source of per-channel settings. Config JSON schemas (exact keys) per kind:
  - `webhook`: `{"url": "<http url>"}` (already implemented — unchanged)
  - `slack`: `{"url": "<incoming webhook url>"}`
  - `telegram`: `{"token": "<bot token>", "chat_id": "<chat id>"}`
  - `ntfy`: `{"base_url": "<server base, default https://ntfy.sh>", "topic": "<topic>", "token": "<optional bearer token, omitted or empty when unused>"}`
  - `pushover`: `{"token": "<application/api token>", "user": "<user or group key>"}`
- Verified external request formats (from live probes 2026-07-12):
  - **ntfy**: `POST {base_url}/{topic}`, body = message text, headers `Title`, `Priority` (`min|low|default|high|max`), `Tags` (comma-separated); optional `Authorization: Bearer <token>`. Real probe returned HTTP 200 + JSON `{"id":...,"event":"message",...}`.
  - **Telegram**: `POST {base}/bot{token}/sendMessage` (base = `https://api.telegram.org`), JSON body `{"chat_id": "...", "text": "..."}`. Auth/argument failures use real HTTP status codes (probe: dummy token → HTTP 401 `{"ok":false,"error_code":401,"description":"Unauthorized"}`). Success → HTTP 200 `{"ok":true,...}`. Treat `resp.status().is_success()` as the success signal.
  - **Slack**: `POST {url}` (the incoming-webhook URL), JSON body `{"text": "..."}`. Success → HTTP 200 body `ok`; failure → non-2xx. Treat `resp.status().is_success()` as the success signal.
  - **Pushover**: `POST https://api.pushover.net/1/messages.json`, `application/x-www-form-urlencoded` body with `token` (app token), `user` (user/group key), `message`; optional `title`, `priority`. Probe with a dummy token → HTTP 400 `{"token":"invalid","errors":[...],"status":0}`; success → HTTP 200 `{"status":1,...}`. Treat `resp.status().is_success()` as the success signal. Base URL is injectable for tests.

---

## File Structure

- `src/models.rs` (MODIFY): add the `Pushover => "pushover"` variant to the `ChannelKind` `str_enum!` and to the round-trip test's variant list.
- `src/notify.rs` (MODIFY): add `http_client()` helper, `event_text()`/`event_title()` message helpers, `TelegramNotifier`, `SlackNotifier`, `NtfyNotifier`, `PushoverNotifier`, and extend `notifier_for`. All new `Notifier` impls + their unit tests live here beside the existing `WebhookNotifier`.
- `src/web.rs` (MODIFY): extend `ChannelForm` with the per-kind optional fields and rewrite `channel_create` to validate + build `config_json` per kind.
- `templates/channel_form.html` (MODIFY): offer all four kinds in the `<select>` and show per-kind config inputs (progressive disclosure via a small inline vanilla-JS toggle).
- `tests/auth_web.rs` (MODIFY): integration test creating a telegram channel through the web handler and asserting the persisted `(kind, config_json)`.

No new files. No migration changes.

---

### Task 1: Shared helpers + TelegramNotifier

**Files:**
- Modify: `src/notify.rs` (add helpers + `TelegramNotifier` + unit tests)

**Interfaces:**
- Consumes: `NotificationEvent { check_id, check_name, event: EventKind, at, project_id }`, `EventKind::{Down,Up}`, `EventKind::as_str`, `NotifyError(pub String)`, `Notifier` trait (all in `src/notify.rs`).
- Produces:
  - `fn http_client() -> reqwest::Client` — builds a reqwest client with a 10s timeout (shared by all notifiers).
  - `fn event_text(ev: &NotificationEvent) -> String` — human-readable one-liner.
  - `fn event_title(ev: &NotificationEvent) -> String` — short title (for ntfy `Title` header).
  - `pub struct TelegramNotifier` with `pub fn new(token: String, chat_id: String) -> Self` and `pub fn with_base_url(token: String, chat_id: String, base_url: String) -> Self`, plus `impl Notifier`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `src/notify.rs`:

```rust
#[tokio::test]
async fn telegram_posts_sendmessage_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/bot123:ABC/sendMessage"))
        .and(body_string_contains("\"chat_id\":\"999\""))
        .and(body_string_contains("DOWN"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"ok\":true}"))
        .expect(1)
        .mount(&server)
        .await;

    let n = TelegramNotifier::with_base_url("123:ABC".into(), "999".into(), server.uri());
    let ev = NotificationEvent {
        check_id: 1,
        check_name: "backup".into(),
        event: EventKind::Down,
        at: Utc::now(),
        project_id: 1,
    };
    n.send(&ev).await.unwrap();
    // wiremock verifies expect(1) on drop
}

#[tokio::test]
async fn telegram_returns_err_on_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("{\"ok\":false}"))
        .mount(&server)
        .await;
    let n = TelegramNotifier::with_base_url("bad".into(), "1".into(), server.uri());
    let ev = NotificationEvent {
        check_id: 1,
        check_name: "backup".into(),
        event: EventKind::Down,
        at: Utc::now(),
        project_id: 1,
    };
    assert!(n.send(&ev).await.is_err());
}
```

The `mod tests` block currently imports `use wiremock::matchers::{method, path};` — extend it to `use wiremock::matchers::{body_string_contains, method, path};`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run --lib notify::tests::telegram`
Expected: FAIL (compile error — `TelegramNotifier` not found).

- [ ] **Step 3: Write the helpers + TelegramNotifier**

In `src/notify.rs`, replace the inline client construction in `WebhookNotifier::new` with the shared helper, and add the helpers + notifier. First add these free functions (place them just above `WebhookNotifier`):

```rust
/// Shared reqwest client: a 10s request timeout keeps a hung endpoint from
/// blocking delivery forever. Falls back to a default client if the builder
/// fails (it never does with these options, but we avoid unwrap-panics).
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// One-line human summary of a state transition, reused by text-oriented
/// channels (Telegram, Slack, and the ntfy body).
fn event_text(ev: &NotificationEvent) -> String {
    let (emoji, word) = match ev.event {
        EventKind::Down => ("\u{1F534}", "DOWN"), // red circle
        EventKind::Up => ("\u{1F7E2}", "UP"),     // green circle
    };
    format!(
        "{emoji} {name} is {word} (as of {at})",
        name = ev.check_name,
        at = ev.at.to_rfc3339()
    )
}

/// Short title for channels with a separate title field (ntfy).
fn event_title(ev: &NotificationEvent) -> String {
    format!("pingward: {} {}", ev.check_name, ev.event.as_str())
}
```

Then change `WebhookNotifier::new` to use `http_client()`:

```rust
impl WebhookNotifier {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: http_client(),
        }
    }
}
```

Then add the Telegram notifier (place it after `WebhookNotifier`'s `impl Notifier`, before the `use crate::models::...` line):

```rust
/// Telegram Bot API. `POST {base_url}/bot{token}/sendMessage` with a JSON
/// `{chat_id, text}` body. `base_url` is injectable so tests can point at a
/// mock server; production uses `https://api.telegram.org`.
pub struct TelegramNotifier {
    token: String,
    chat_id: String,
    base_url: String,
    client: reqwest::Client,
}

impl TelegramNotifier {
    pub fn new(token: String, chat_id: String) -> Self {
        Self::with_base_url(token, chat_id, "https://api.telegram.org".to_string())
    }

    pub fn with_base_url(token: String, chat_id: String, base_url: String) -> Self {
        Self {
            token,
            chat_id,
            base_url: base_url.trim_end_matches('/').to_string(),
            client: http_client(),
        }
    }
}

impl Notifier for TelegramNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/bot{}/sendMessage", self.base_url, self.token);
            let body = serde_json::json!({
                "chat_id": self.chat_id,
                "text": event_text(ev),
            });
            let resp = self
                .client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| NotifyError(e.to_string()))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(NotifyError(format!("status {}", resp.status())))
            }
        })
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run --lib notify::tests`
Expected: PASS (all existing notify tests + the two new telegram tests).

- [ ] **Step 5: fmt + clippy**

Run: `cd /Users/henry/Develop/claude/pingward && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: no changes reported by fmt beyond formatting, clippy clean.

- [ ] **Step 6: Commit**

```bash
git add src/notify.rs
git commit -m "feat: TelegramNotifier + shared http_client/message helpers"
```

---

### Task 2: SlackNotifier

**Files:**
- Modify: `src/notify.rs` (add `SlackNotifier` + unit test)

**Interfaces:**
- Consumes: `http_client`, `event_text`, `Notifier`, `NotifyError`, `NotificationEvent` (Task 1).
- Produces: `pub struct SlackNotifier` with `pub fn new(url: String) -> Self` + `impl Notifier`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/notify.rs`:

```rust
#[tokio::test]
async fn slack_posts_text_json() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/services/hook"))
        .and(body_string_contains("\"text\""))
        .and(body_string_contains("UP"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .expect(1)
        .mount(&server)
        .await;

    let n = SlackNotifier::new(format!("{}/services/hook", server.uri()));
    let ev = NotificationEvent {
        check_id: 1,
        check_name: "backup".into(),
        event: EventKind::Up,
        at: Utc::now(),
        project_id: 1,
    };
    n.send(&ev).await.unwrap();
}

#[tokio::test]
async fn slack_returns_err_on_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let n = SlackNotifier::new(format!("{}/x", server.uri()));
    let ev = NotificationEvent {
        check_id: 1,
        check_name: "backup".into(),
        event: EventKind::Up,
        at: Utc::now(),
        project_id: 1,
    };
    assert!(n.send(&ev).await.is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run --lib notify::tests::slack`
Expected: FAIL (compile error — `SlackNotifier` not found).

- [ ] **Step 3: Write SlackNotifier**

Add after `TelegramNotifier`'s `impl Notifier` in `src/notify.rs`:

```rust
/// Slack incoming webhook: `POST {url}` with a JSON `{text}` body.
pub struct SlackNotifier {
    url: String,
    client: reqwest::Client,
}

impl SlackNotifier {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: http_client(),
        }
    }
}

impl Notifier for SlackNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let body = serde_json::json!({ "text": event_text(ev) });
            let resp = self
                .client
                .post(&self.url)
                .json(&body)
                .send()
                .await
                .map_err(|e| NotifyError(e.to_string()))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(NotifyError(format!("status {}", resp.status())))
            }
        })
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run --lib notify::tests`
Expected: PASS.

- [ ] **Step 5: fmt + clippy**

Run: `cd /Users/henry/Develop/claude/pingward && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/notify.rs
git commit -m "feat: SlackNotifier (incoming webhook)"
```

---

### Task 3: NtfyNotifier

**Files:**
- Modify: `src/notify.rs` (add `NtfyNotifier` + unit test)

**Interfaces:**
- Consumes: `http_client`, `event_text`, `event_title`, `Notifier`, `NotifyError`, `NotificationEvent`, `EventKind` (Task 1).
- Produces: `pub struct NtfyNotifier` with `pub fn new(base_url: String, topic: String, token: Option<String>) -> Self` + `impl Notifier`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/notify.rs`. This asserts the URL path (base+topic), the `Title` header, and that a bearer token is forwarded:

```rust
#[tokio::test]
async fn ntfy_posts_body_with_headers_and_token() {
    use wiremock::matchers::header;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/mytopic"))
        .and(header("authorization", "Bearer tok"))
        .and(header("priority", "high"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"id\":\"x\"}"))
        .expect(1)
        .mount(&server)
        .await;

    let n = NtfyNotifier::new(server.uri(), "mytopic".into(), Some("tok".into()));
    let ev = NotificationEvent {
        check_id: 1,
        check_name: "backup".into(),
        event: EventKind::Down,
        at: Utc::now(),
        project_id: 1,
    };
    n.send(&ev).await.unwrap();
}

#[tokio::test]
async fn ntfy_works_without_token_and_errors_on_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let n = NtfyNotifier::new(server.uri(), "t".into(), None);
    let ev = NotificationEvent {
        check_id: 1,
        check_name: "backup".into(),
        event: EventKind::Up,
        at: Utc::now(),
        project_id: 1,
    };
    assert!(n.send(&ev).await.is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run --lib notify::tests::ntfy`
Expected: FAIL (compile error — `NtfyNotifier` not found).

- [ ] **Step 3: Write NtfyNotifier**

Add after `SlackNotifier`'s `impl Notifier` in `src/notify.rs`:

```rust
/// ntfy publish: `POST {base_url}/{topic}` with the message as the body and
/// `Title`/`Priority`/`Tags` headers. An optional bearer token authenticates
/// against protected topics / self-hosted servers.
pub struct NtfyNotifier {
    base_url: String,
    topic: String,
    token: Option<String>,
    client: reqwest::Client,
}

impl NtfyNotifier {
    pub fn new(base_url: String, topic: String, token: Option<String>) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            topic,
            token: token.filter(|t| !t.is_empty()),
            client: http_client(),
        }
    }
}

impl Notifier for NtfyNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/{}", self.base_url, self.topic);
            let (priority, tags) = match ev.event {
                EventKind::Down => ("high", "red_circle"),
                EventKind::Up => ("default", "green_circle"),
            };
            let mut req = self
                .client
                .post(&url)
                .header("Title", event_title(ev))
                .header("Priority", priority)
                .header("Tags", tags)
                .body(event_text(ev));
            if let Some(t) = &self.token {
                req = req.bearer_auth(t);
            }
            let resp = req.send().await.map_err(|e| NotifyError(e.to_string()))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(NotifyError(format!("status {}", resp.status())))
            }
        })
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run --lib notify::tests`
Expected: PASS.

- [ ] **Step 5: fmt + clippy**

Run: `cd /Users/henry/Develop/claude/pingward && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/notify.rs
git commit -m "feat: NtfyNotifier (body + Title/Priority/Tags headers, optional bearer)"
```

---

### Task 4: PushoverNotifier + ChannelKind::Pushover variant

**Files:**
- Modify: `src/models.rs` (`ChannelKind` variant + round-trip test)
- Modify: `src/notify.rs` (add `PushoverNotifier` + unit tests)

**Interfaces:**
- Consumes: `http_client`, `event_text`, `event_title`, `Notifier`, `NotifyError`, `NotificationEvent` (Task 1).
- Produces:
  - `ChannelKind::Pushover` (round-trips to/from `"pushover"`).
  - `pub struct PushoverNotifier` with `pub fn new(token: String, user: String) -> Self` and `pub fn with_base_url(token: String, user: String, base_url: String) -> Self`, plus `impl Notifier`.

- [ ] **Step 1: Add the ChannelKind variant + extend its test**

In `src/models.rs`, add `Pushover => "pushover"` to the `ChannelKind` `str_enum!` (line 25):

```rust
str_enum!(ChannelKind { Webhook => "webhook", Telegram => "telegram", Slack => "slack", Ntfy => "ntfy", Pushover => "pushover" });
```

And add `ChannelKind::Pushover,` to the round-trip test's array (currently `src/models.rs:121-126`):

```rust
        for k in [
            ChannelKind::Webhook,
            ChannelKind::Telegram,
            ChannelKind::Slack,
            ChannelKind::Ntfy,
            ChannelKind::Pushover,
        ] {
```

- [ ] **Step 2: Write the failing notifier test**

Add to `mod tests` in `src/notify.rs`:

```rust
#[tokio::test]
async fn pushover_posts_form_with_token_and_user() {
    use wiremock::matchers::body_string_contains;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/1/messages.json"))
        .and(body_string_contains("token=apptok"))
        .and(body_string_contains("user=userkey"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"status\":1}"))
        .expect(1)
        .mount(&server)
        .await;

    let n = PushoverNotifier::with_base_url("apptok".into(), "userkey".into(), server.uri());
    let ev = NotificationEvent {
        check_id: 1,
        check_name: "backup".into(),
        event: EventKind::Down,
        at: Utc::now(),
        project_id: 1,
    };
    n.send(&ev).await.unwrap();
}

#[tokio::test]
async fn pushover_returns_err_on_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_string("{\"status\":0}"))
        .mount(&server)
        .await;
    let n = PushoverNotifier::with_base_url("bad".into(), "bad".into(), server.uri());
    let ev = NotificationEvent {
        check_id: 1,
        check_name: "backup".into(),
        event: EventKind::Up,
        at: Utc::now(),
        project_id: 1,
    };
    assert!(n.send(&ev).await.is_err());
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run --lib pushover models::tests::channel_kind`
Expected: FAIL (compile error — `PushoverNotifier` not found; the models test now references `ChannelKind::Pushover` which exists after Step 1, so that half compiles).

- [ ] **Step 4: Write PushoverNotifier**

Add after `NtfyNotifier`'s `impl Notifier` in `src/notify.rs`. Pushover expects a form-urlencoded body; reqwest's `.form(&[...])` sets the content type and encoding:

```rust
/// Pushover: `POST {base_url}/1/messages.json` with a form body carrying the
/// app `token`, the recipient `user` key, and the `message`. `base_url` is
/// injectable so tests can point at a mock; production uses
/// `https://api.pushover.net`.
pub struct PushoverNotifier {
    token: String,
    user: String,
    base_url: String,
    client: reqwest::Client,
}

impl PushoverNotifier {
    pub fn new(token: String, user: String) -> Self {
        Self::with_base_url(token, user, "https://api.pushover.net".to_string())
    }

    pub fn with_base_url(token: String, user: String, base_url: String) -> Self {
        Self {
            token,
            user,
            base_url: base_url.trim_end_matches('/').to_string(),
            client: http_client(),
        }
    }
}

impl Notifier for PushoverNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}/1/messages.json", self.base_url);
            let priority = match ev.event {
                EventKind::Down => "1",
                EventKind::Up => "0",
            };
            let resp = self
                .client
                .post(&url)
                .form(&[
                    ("token", self.token.as_str()),
                    ("user", self.user.as_str()),
                    ("title", event_title(ev).as_str()),
                    ("message", event_text(ev).as_str()),
                    ("priority", priority),
                ])
                .send()
                .await
                .map_err(|e| NotifyError(e.to_string()))?;
            if resp.status().is_success() {
                Ok(())
            } else {
                Err(NotifyError(format!("status {}", resp.status())))
            }
        })
    }
}
```

Note: `event_title(ev)` and `event_text(ev)` each return an owned `String`; binding them to `.as_str()` inside the array is a temporary-lifetime error. Bind them to locals first:

```rust
            let title = event_title(ev);
            let message = event_text(ev);
            let resp = self
                .client
                .post(&url)
                .form(&[
                    ("token", self.token.as_str()),
                    ("user", self.user.as_str()),
                    ("title", title.as_str()),
                    ("message", message.as_str()),
                    ("priority", priority),
                ])
                .send()
                .await
                .map_err(|e| NotifyError(e.to_string()))?;
```

Use this second form (locals bound before the array).

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run --lib notify::tests pushover models::tests::channel_kind`
Expected: PASS.

- [ ] **Step 6: fmt + clippy**

Run: `cd /Users/henry/Develop/claude/pingward && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/models.rs src/notify.rs
git commit -m "feat: PushoverNotifier + ChannelKind::Pushover"
```

---

### Task 5: Wire notifier_for for all kinds

**Files:**
- Modify: `src/notify.rs` (`notifier_for` match + unit tests)

**Interfaces:**
- Consumes: `TelegramNotifier`, `SlackNotifier`, `NtfyNotifier`, `PushoverNotifier` (Tasks 1-4), `Channel { id, project_id, kind: ChannelKind, name, config_json, created_at }`, `ChannelKind::{Webhook,Telegram,Slack,Ntfy,Pushover}`.
- Produces: `notifier_for(&Channel) -> Option<Box<dyn Notifier>>` returning a live notifier for every kind whose config is valid, `None` (with a `warn` log) when required config keys are missing/blank.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `src/notify.rs`. These build `Channel` values directly (no DB) and assert `notifier_for` returns `Some`/`None` as expected:

```rust
fn channel_with(kind: ChannelKind, config_json: &str) -> Channel {
    Channel {
        id: 1,
        project_id: 1,
        kind,
        name: "c".into(),
        config_json: config_json.into(),
        created_at: Utc::now(),
    }
}

#[test]
fn notifier_for_builds_each_kind_with_valid_config() {
    assert!(notifier_for(&channel_with(ChannelKind::Webhook, "{\"url\":\"http://x\"}")).is_some());
    assert!(notifier_for(&channel_with(ChannelKind::Slack, "{\"url\":\"http://x\"}")).is_some());
    assert!(notifier_for(&channel_with(
        ChannelKind::Telegram,
        "{\"token\":\"t\",\"chat_id\":\"1\"}"
    ))
    .is_some());
    assert!(notifier_for(&channel_with(
        ChannelKind::Ntfy,
        "{\"base_url\":\"https://ntfy.sh\",\"topic\":\"t\"}"
    ))
    .is_some());
    assert!(notifier_for(&channel_with(
        ChannelKind::Pushover,
        "{\"token\":\"t\",\"user\":\"u\"}"
    ))
    .is_some());
}

#[test]
fn notifier_for_returns_none_on_missing_config() {
    assert!(notifier_for(&channel_with(ChannelKind::Slack, "{}")).is_none());
    assert!(notifier_for(&channel_with(ChannelKind::Telegram, "{\"token\":\"t\"}")).is_none());
    assert!(notifier_for(&channel_with(ChannelKind::Ntfy, "{\"base_url\":\"x\"}")).is_none());
    assert!(notifier_for(&channel_with(ChannelKind::Pushover, "{\"token\":\"t\"}")).is_none());
}
```

`Channel` and `ChannelKind` are already imported at module scope (`use crate::models::{Channel, ChannelKind, NotifyStatus};`) but the `mod tests` block only imports `ChannelKind`. Add `use crate::models::Channel;` inside `mod tests` if the test does not compile (the `use super::*;` at the top of `mod tests` already re-exports the module-level `Channel` import, so no change should be needed — verify by compiling).

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run --lib notify::tests::notifier_for`
Expected: FAIL — `notifier_for_builds_each_kind_with_valid_config` fails because Telegram/Slack/Ntfy/Pushover currently hit the `other => None` catch-all arm.

- [ ] **Step 3: Replace the notifier_for match**

In `src/notify.rs`, replace the whole `notifier_for` function body. Add a small config-reading helper above it, then the expanded match:

```rust
/// Read a required non-empty string field from parsed channel config.
fn cfg_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Build a notifier for a channel from its `(kind, config_json)`. Returns
/// `None` (with a warning) when a required config field is missing or blank —
/// `deliver_event` skips such channels rather than failing the event.
pub fn notifier_for(channel: &Channel) -> Option<Box<dyn Notifier>> {
    let cfg: serde_json::Value = serde_json::from_str(&channel.config_json)
        .map_err(|e| {
            tracing::warn!(channel_id = channel.id, "invalid config_json: {e}");
        })
        .ok()?;
    let missing = |field: &str| {
        tracing::warn!(
            channel_id = channel.id,
            kind = channel.kind.as_str(),
            "channel missing required config field: {field}"
        );
        None::<Box<dyn Notifier>>
    };
    match channel.kind {
        ChannelKind::Webhook => match cfg_str(&cfg, "url") {
            Some(url) => Some(Box::new(WebhookNotifier::new(url))),
            None => missing("url"),
        },
        ChannelKind::Slack => match cfg_str(&cfg, "url") {
            Some(url) => Some(Box::new(SlackNotifier::new(url))),
            None => missing("url"),
        },
        ChannelKind::Telegram => match (cfg_str(&cfg, "token"), cfg_str(&cfg, "chat_id")) {
            (Some(token), Some(chat_id)) => Some(Box::new(TelegramNotifier::new(token, chat_id))),
            _ => missing("token/chat_id"),
        },
        ChannelKind::Ntfy => match cfg_str(&cfg, "topic") {
            Some(topic) => {
                let base_url =
                    cfg_str(&cfg, "base_url").unwrap_or_else(|| "https://ntfy.sh".to_string());
                let token = cfg_str(&cfg, "token");
                Some(Box::new(NtfyNotifier::new(base_url, topic, token)))
            }
            None => missing("topic"),
        },
        ChannelKind::Pushover => match (cfg_str(&cfg, "token"), cfg_str(&cfg, "user")) {
            (Some(token), Some(user)) => Some(Box::new(PushoverNotifier::new(token, user))),
            _ => missing("token/user"),
        },
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run --lib notify::tests`
Expected: PASS (all notify unit tests including the two new `notifier_for_*`).

- [ ] **Step 5: fmt + clippy**

Run: `cd /Users/henry/Develop/claude/pingward && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add src/notify.rs
git commit -m "feat: notifier_for builds telegram/slack/ntfy from config_json"
```

---

### Task 6: Channel-creation UI + handler for all kinds

**Files:**
- Modify: `templates/channel_form.html`
- Modify: `src/web.rs` (`ChannelForm` struct + `channel_create` handler)
- Test: `tests/auth_web.rs`

**Interfaces:**
- Consumes: `ChannelKind::{Webhook,Slack,Telegram,Ntfy,Pushover}`, `ChannelKind::from_str` (via `std::str::FromStr`), `ChannelKind::as_str`, `Store::create_channel(project_id, kind, name, config_json, now)`, `render`, `ChannelFormTemplate { show_nav, project_id, error }`, `owned_project`, `CurrentUser`.
- Produces: a `channel_create` that persists a valid `(kind, config_json)` for any of the four kinds, or re-renders the form with an error message on invalid/blank required fields.

- [ ] **Step 1: Write the failing test**

Add to `tests/auth_web.rs`. Follow the existing helper/style in that file (a logged-in admin session with a project). If the file already has a helper that returns an authenticated `TestServer` + project id, reuse it; otherwise mirror the existing channel test's setup. The assertion is that POSTing a telegram channel persists the right kind + config. Concretely:

```rust
#[tokio::test]
async fn create_telegram_channel_persists_config() {
    // `setup_logged_in_with_project` mirrors the existing auth_web helpers:
    // it runs setup, logs in, creates a project, and returns (server, pid, store).
    let (server, pid, store) = setup_logged_in_with_project().await;

    let res = server
        .post(&format!("/projects/{pid}/channels"))
        .form(&[
            ("name", "tg"),
            ("kind", "telegram"),
            ("telegram_token", "123:ABC"),
            ("telegram_chat_id", "999"),
        ])
        .await;
    assert_eq!(res.status_code(), 303); // redirect to /projects/{pid}

    let channels = store.channels_for_project(pid).await.unwrap();
    let tg = channels
        .iter()
        .find(|c| c.kind == pingward::models::ChannelKind::Telegram)
        .expect("telegram channel persisted");
    assert!(tg.config_json.contains("\"token\":\"123:ABC\""));
    assert!(tg.config_json.contains("\"chat_id\":\"999\""));
}
```

If `channels_for_project` does not exist on `Store`, assert via whatever channel-listing query the existing channel test already uses (check `tests/auth_web.rs` and `src/store.rs` for the name — e.g. `list_channels(pid)`); do NOT add a new store method just for the test. Match the redirect status the existing `channel_create` returns (Plan 2 used `Redirect::to`, which is 303 See Other under axum 0.8 — confirm against an existing redirect assertion in `tests/auth_web.rs` and use the same code).

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run --test auth_web create_telegram_channel_persists_config`
Expected: FAIL — the current `channel_create` rejects any non-webhook kind (`form.kind != ChannelKind::Webhook.as_str()`).

- [ ] **Step 3: Extend the ChannelForm struct**

In `src/web.rs`, replace the `ChannelForm` struct (currently `name`, `kind`, `url`):

Each kind gets its OWN uniquely-named fields (not shared `url`/`token`). Shared names are unsafe here: the form renders every kind's inputs and the browser submits all of them, so a duplicate key like `url` appearing twice (webhook's filled value, then slack's empty value) lets the empty later field clobber the filled earlier one under `serde_urlencoded`. Unique names make each field unambiguous regardless of which group is visible.

```rust
#[derive(Deserialize)]
struct ChannelForm {
    name: String,
    kind: String,
    #[serde(default)]
    webhook_url: String,
    #[serde(default)]
    slack_url: String,
    #[serde(default)]
    telegram_token: String,
    #[serde(default)]
    telegram_chat_id: String,
    #[serde(default)]
    ntfy_base_url: String, // optional, defaults to https://ntfy.sh
    #[serde(default)]
    ntfy_topic: String,
    #[serde(default)]
    ntfy_token: String, // optional
    #[serde(default)]
    pushover_token: String, // application token
    #[serde(default)]
    pushover_user: String, // user/group key
}
```

- [ ] **Step 4: Rewrite channel_create**

Replace the body of `channel_create` in `src/web.rs`. Parse the kind, validate the required fields for that kind, build the matching `config_json`, and on any validation failure re-render the form with an error:

```rust
async fn channel_create(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(pid): Path<i64>,
    Form(form): Form<ChannelForm>,
) -> Result<Response, AppError> {
    owned_project(&state.store, pid, user.id).await?;

    let err = |msg: &str| -> Result<Response, AppError> {
        Ok(render(&ChannelFormTemplate {
            show_nav: true,
            project_id: pid,
            error: Some(msg.to_string()),
        })?
        .into_response())
    };

    let name = form.name.trim();
    if name.is_empty() {
        return err("a channel name is required");
    }

    let Ok(kind) = ChannelKind::from_str(&form.kind) else {
        return err("unknown channel kind");
    };

    let config = match kind {
        ChannelKind::Webhook => {
            let url = form.webhook_url.trim();
            if url.is_empty() {
                return err("a webhook URL is required");
            }
            serde_json::json!({ "url": url }).to_string()
        }
        ChannelKind::Slack => {
            let url = form.slack_url.trim();
            if url.is_empty() {
                return err("a Slack incoming-webhook URL is required");
            }
            serde_json::json!({ "url": url }).to_string()
        }
        ChannelKind::Telegram => {
            let token = form.telegram_token.trim();
            let chat_id = form.telegram_chat_id.trim();
            if token.is_empty() || chat_id.is_empty() {
                return err("Telegram requires both a bot token and a chat id");
            }
            serde_json::json!({ "token": token, "chat_id": chat_id }).to_string()
        }
        ChannelKind::Ntfy => {
            let topic = form.ntfy_topic.trim();
            if topic.is_empty() {
                return err("ntfy requires a topic");
            }
            let base_url = {
                let b = form.ntfy_base_url.trim();
                if b.is_empty() { "https://ntfy.sh" } else { b }
            };
            let token = form.ntfy_token.trim();
            serde_json::json!({
                "base_url": base_url,
                "topic": topic,
                "token": token,
            })
            .to_string()
        }
        ChannelKind::Pushover => {
            let token = form.pushover_token.trim();
            let user = form.pushover_user.trim();
            if token.is_empty() || user.is_empty() {
                return err("Pushover requires both an application token and a user key");
            }
            serde_json::json!({ "token": token, "user": user }).to_string()
        }
    };

    state
        .store
        .create_channel(pid, kind, name, &config, Utc::now())
        .await?;
    Ok(Redirect::to(&format!("/projects/{pid}")).into_response())
}
```

Ensure `use std::str::FromStr;` is in scope in `src/web.rs` (add it to the imports if the file does not already import it — `ChannelKind::from_str` requires the trait in scope).

- [ ] **Step 5: Expand the channel form template**

Replace `templates/channel_form.html` with a form that offers all four kinds and shows the fields for the selected kind. A small inline vanilla-JS toggle (spec §2 permits vanilla JS, no build step) shows only the relevant field group; server-side validation in Step 4 is the source of truth regardless of JS:

```html
{% extends "base.html" %}
{% block body %}
<h1>New channel</h1>
{% if let Some(error) = error %}<p class="status-down">{{ error }}</p>{% endif %}
<form method="post" action="/projects/{{ project_id }}/channels">
  <label>Name <input name="name" required></label>
  <label>Kind
    <select name="kind" id="kind">
      <option value="webhook">webhook</option>
      <option value="slack">slack</option>
      <option value="telegram">telegram</option>
      <option value="ntfy">ntfy</option>
      <option value="pushover">pushover</option>
    </select>
  </label>

  <div class="cfg" data-kind="webhook">
    <label>Webhook URL <input name="webhook_url"></label>
  </div>
  <div class="cfg" data-kind="slack">
    <label>Slack incoming webhook URL <input name="slack_url"></label>
  </div>
  <div class="cfg" data-kind="telegram">
    <label>Bot token <input name="telegram_token"></label>
    <label>Chat id <input name="telegram_chat_id"></label>
  </div>
  <div class="cfg" data-kind="ntfy">
    <label>Server base URL <input name="ntfy_base_url" placeholder="https://ntfy.sh"></label>
    <label>Topic <input name="ntfy_topic"></label>
    <label>Token (optional) <input name="ntfy_token"></label>
  </div>
  <div class="cfg" data-kind="pushover">
    <label>Application token <input name="pushover_token"></label>
    <label>User/group key <input name="pushover_user"></label>
  </div>

  <button type="submit">Create</button>
</form>
<script>
  (function () {
    var sel = document.getElementById('kind');
    function sync() {
      var k = sel.value;
      document.querySelectorAll('.cfg').forEach(function (d) {
        d.style.display = d.getAttribute('data-kind') === k ? '' : 'none';
      });
    }
    sel.addEventListener('change', sync);
    sync();
  })();
</script>
{% endblock %}
```

Note: every input has a unique `name` (Step 3's struct fields), so the always-submitted hidden groups never collide with the visible one. Do NOT mark these inputs `required` — a hidden `required` field blocks submission in some browsers.

- [ ] **Step 6: Run the integration test + full suite**

Run: `cd /Users/henry/Develop/claude/pingward && cargo nextest run`
Expected: PASS — the new telegram test plus the entire existing suite.

- [ ] **Step 7: fmt + clippy**

Run: `cd /Users/henry/Develop/claude/pingward && cargo fmt && cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add src/web.rs templates/channel_form.html tests/auth_web.rs
git commit -m "feat: create telegram/slack/ntfy channels via web UI"
```

---

## Self-Review

**1. Spec coverage (spec §11 Notification Channels, plus user-requested Pushover):**
- Webhook — unchanged (Plan 2). ✅
- Telegram (bot token + chat id) — Task 1 notifier, Task 5 wiring, Task 6 UI. ✅
- Slack (incoming webhook URL) — Task 2 notifier, Task 5 wiring, Task 6 UI. ✅
- ntfy (server + topic + optional token) — Task 3 notifier, Task 5 wiring, Task 6 UI. ✅
- Pushover (app token + user key) — Task 4 notifier + `ChannelKind::Pushover`, Task 5 wiring, Task 6 UI. ✅ (User-requested addition beyond spec §11; validated by live API probe.)
- "Pluggable behind a `Notifier` trait; dispatched via an enum" — every kind is a `Notifier` impl selected by `notifier_for`'s `match channel.kind`. ✅
- "Adding email later requires only a new impl + variant + UI option" — Pushover was added exactly this way (impl + `ChannelKind` variant + match arm + form group), demonstrating the extension point holds. ✅

**2. Placeholder scan:** every code step contains complete, compilable code — notifiers, helpers, the full `notifier_for` match, the full `channel_create`, and the full template. No TODOs. The only deliberately conditional instructions are the two test-side "confirm the existing name" notes in Task 5 (redirect status code; channel-listing store method), which exist because those names are defined in Plan 2 code the implementer must read rather than guess — the implementer is told exactly where to look and what to match.

**3. Type consistency:**
- `Notifier::send` signature (`Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>>`) is copied verbatim from the existing `WebhookNotifier` impl across all four new notifiers. ✅
- `http_client() -> reqwest::Client` produced in Task 1, consumed by Tasks 2-4. ✅
- `event_text`/`event_title` produced in Task 1, consumed by Tasks 2-4. ✅
- `TelegramNotifier::new` / `SlackNotifier::new` / `NtfyNotifier::new` / `PushoverNotifier::new` signatures match their `notifier_for` call sites in Task 5. ✅
- `cfg_str` returns `Option<String>`; every call site treats a `None` as "missing required field". ✅
- Config JSON keys written by `channel_create` (Task 6) exactly match the keys read by `notifier_for` (Task 5): webhook/slack `url`; telegram `token`+`chat_id`; ntfy `base_url`+`topic`+`token`; pushover `token`+`user`. ✅
- Form field names (Task 6 `ChannelForm`) are unique per kind, so hidden groups never clobber the visible one; the handler reads the field matching the selected `kind`. ✅
