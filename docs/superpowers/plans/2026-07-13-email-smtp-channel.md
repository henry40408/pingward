# Email (SMTP) Notification Channel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an Email (SMTP) notification channel, with instance-level SMTP config via environment variables and a per-channel recipient address.

**Architecture:** Model A — pingward is an SMTP client to an operator-provided relay. SMTP connection settings live in `Config.smtp: Option<SmtpConfig>` (parsed from `PINGWARD_SMTP_*` env); an Email channel stores only `{"to": address}`. The `smtp` config is threaded to the three notifier call sites (web send-test, ping ingest, scheduler). An unconfigured/dangling Email channel surfaces as a recorded delivery error, never a crash.

**Tech Stack:** Rust, axum 0.8, sqlx 0.9 (Any), askama 0.16, `lettre` 0.11 (async SMTP), tests via `cargo nextest` + `axum-test` 21.

## Global Constraints

- Tests run with `cargo nextest run` (never `cargo test`).
- `cargo fmt` before every commit; `cargo clippy --all-targets -- -D warnings` must pass.
- All commits GPG-signed; end message with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- Dependency cooldown: pin `lettre = "0.11"` (Cargo.lock resolves 0.11.22, published 2026-05-14 — >=7 days old). Do NOT adopt a version published <7 days ago.
- No database schema/migration change; events stay `down`/`up`/`reminder`.
- `smtp` is `Some` only when both `PINGWARD_SMTP_HOST` and `PINGWARD_SMTP_FROM` are set; AUTH only when both username and password are set.
- Redaction discipline (from #18): never include credentials in a `NotifyError`.

---

### Task 1: `SmtpConfig` + env parsing

**Files:**
- Modify: `src/config.rs` (add `SmtpTls`, `SmtpConfig`, `Config.smtp`, parse in `from_map`)
- Test: `src/config.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Produces: `pub enum SmtpTls { Starttls, Tls, None }`; `pub struct SmtpConfig { host: String, port: u16, username: Option<String>, password: Option<String>, from: String, tls: SmtpTls }`; `Config.smtp: Option<SmtpConfig>`.

- [ ] **Step 1: Write the failing tests**

Add to `src/config.rs` tests module:

```rust
    #[test]
    fn smtp_none_when_host_or_from_missing() {
        assert!(Config::from_map(|_| None).smtp.is_none());
        let c = Config::from_map(|k| (k == "PINGWARD_SMTP_HOST").then(|| "mail.x".into()));
        assert!(c.smtp.is_none());
        let c = Config::from_map(|k| (k == "PINGWARD_SMTP_FROM").then(|| "a@x".into()));
        assert!(c.smtp.is_none());
    }

    #[test]
    fn smtp_parsed_when_host_and_from_present() {
        let c = Config::from_map(|k| match k {
            "PINGWARD_SMTP_HOST" => Some("mail.example.com".into()),
            "PINGWARD_SMTP_FROM" => Some("alerts@example.com".into()),
            "PINGWARD_SMTP_PORT" => Some("2525".into()),
            "PINGWARD_SMTP_USERNAME" => Some("u".into()),
            "PINGWARD_SMTP_PASSWORD" => Some("p".into()),
            "PINGWARD_SMTP_TLS" => Some("TLS".into()),
            _ => None,
        });
        let s = c.smtp.expect("smtp should be Some");
        assert_eq!(s.host, "mail.example.com");
        assert_eq!(s.from, "alerts@example.com");
        assert_eq!(s.port, 2525);
        assert_eq!(s.username.as_deref(), Some("u"));
        assert_eq!(s.password.as_deref(), Some("p"));
        assert_eq!(s.tls, SmtpTls::Tls);
    }

    #[test]
    fn smtp_defaults_port_and_tls_and_optional_auth() {
        let c = Config::from_map(|k| match k {
            "PINGWARD_SMTP_HOST" => Some("mail.x".into()),
            "PINGWARD_SMTP_FROM" => Some("a@x".into()),
            "PINGWARD_SMTP_PORT" => Some("not-a-number".into()),
            "PINGWARD_SMTP_TLS" => Some("weird".into()),
            _ => None,
        });
        let s = c.smtp.unwrap();
        assert_eq!(s.port, 587, "blank/invalid port falls back to 587");
        assert_eq!(s.tls, SmtpTls::Starttls, "unknown tls falls back to starttls");
        assert!(s.username.is_none() && s.password.is_none());
    }

    #[test]
    fn smtp_tls_modes_parse() {
        let mk = |tls: &str| {
            Config::from_map(|k| match k {
                "PINGWARD_SMTP_HOST" => Some("h".into()),
                "PINGWARD_SMTP_FROM" => Some("a@x".into()),
                "PINGWARD_SMTP_TLS" => Some(tls.into()),
                _ => None,
            })
            .smtp
            .unwrap()
            .tls
        };
        assert_eq!(mk("starttls"), SmtpTls::Starttls);
        assert_eq!(mk("none"), SmtpTls::None);
        assert_eq!(mk("tls"), SmtpTls::Tls);
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --lib config::tests::smtp_none_when_host_or_from_missing`
Expected: FAIL to compile — `SmtpTls`/`Config.smtp` do not exist.

- [ ] **Step 3: Add the types and parsing**

In `src/config.rs`, add above `Config`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls {
    Starttls,
    Tls,
    None,
}

#[derive(Debug, Clone)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
    pub tls: SmtpTls,
}
```

Add `pub smtp: Option<SmtpConfig>,` as the last field of `Config`.

In `from_map`, before building `Config`, add:

```rust
        // Instance SMTP: present only when both host and from are set. Any
        // partial config (host without from, etc.) means email is unavailable.
        let nonblank = |k: &str| get(k).map(|v| v.trim().to_string()).filter(|s| !s.is_empty());
        let smtp = match (nonblank("PINGWARD_SMTP_HOST"), nonblank("PINGWARD_SMTP_FROM")) {
            (Some(host), Some(from)) => {
                let port = nonblank("PINGWARD_SMTP_PORT")
                    .and_then(|v| v.parse::<u16>().ok())
                    .unwrap_or(587);
                let tls = match nonblank("PINGWARD_SMTP_TLS")
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str()
                {
                    "tls" => SmtpTls::Tls,
                    "none" => SmtpTls::None,
                    _ => SmtpTls::Starttls,
                };
                Some(SmtpConfig {
                    host,
                    port,
                    username: nonblank("PINGWARD_SMTP_USERNAME"),
                    password: nonblank("PINGWARD_SMTP_PASSWORD"),
                    from,
                    tls,
                })
            }
            _ => None,
        };
```

Add `smtp,` to the `Config { … }` initializer.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run --lib config`
Expected: PASS (new + existing config tests).

- [ ] **Step 5: fmt + clippy + commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings
git add src/config.rs
git commit -m "feat: parse instance SMTP config from PINGWARD_SMTP_* env

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Thread `smtp` through the notifier call sites

**Files:**
- Modify: `src/state.rs` (add `FromRef<AppState> for Arc<Config>`)
- Modify: `src/notify.rs` (`notifier_for`, `deliver_event` signatures + call-site/test updates)
- Modify: `src/scheduler.rs` (`run_scan_loop` gains `smtp`; `deliver_event` calls)
- Modify: `src/ping.rs` (handlers extract config; `apply`/`spawn_delivery` gain `smtp`)
- Modify: `src/web.rs` (`channel_test`'s `notifier_for` call gains `smtp`)
- Modify: `src/main.rs` (pass `config.smtp.clone()` to `run_scan_loop`)

**Interfaces:**
- Consumes: `SmtpConfig` (Task 1).
- Produces: `notifier_for(channel: &Channel, _smtp: Option<&SmtpConfig>)`; `deliver_event(store, ev, policy, now, smtp: Option<&SmtpConfig>)`; `run_scan_loop(store, env_default_secs, smtp: Option<SmtpConfig>)`. No behavior change — every existing channel arm ignores `smtp`.

- [ ] **Step 1: Add `FromRef<AppState> for Arc<Config>`**

In `src/state.rs`, after the `FromRef<AppState> for Store` impl, add:

```rust
impl FromRef<AppState> for Arc<Config> {
    fn from_ref(state: &AppState) -> Arc<Config> {
        state.config.clone()
    }
}
```

- [ ] **Step 2: Change `notifier_for` and `deliver_event` signatures in `src/notify.rs`**

Add to the imports near `use crate::models::{...}`:

```rust
use crate::config::SmtpConfig;
```

Change `notifier_for`'s signature to:

```rust
pub fn notifier_for(channel: &Channel, _smtp: Option<&SmtpConfig>) -> Option<Box<dyn Notifier>> {
```

(The param is unused this task; Task 3 renames it to `smtp` and uses it in the Email arm.)

Change `deliver_event`'s signature to add a trailing param:

```rust
pub async fn deliver_event(
    store: &Store,
    ev: &NotificationEvent,
    policy: RetryPolicy,
    now: DateTime<Utc>,
    smtp: Option<&SmtpConfig>,
) {
```

Inside `deliver_event`, change the notifier build line to:

```rust
        let Some(notifier) = notifier_for(channel, smtp) else {
```

- [ ] **Step 3: Update `notify.rs` test call sites**

In `src/notify.rs` tests, update every `notifier_for(...)` call to pass `None` and every `deliver_event(...)` call to pass a trailing `None`:

- `notifier_for_builds_each_kind_with_valid_config` and `notifier_for_returns_none_on_missing_config`: add `, None` to each `notifier_for(&channel_with(...))` call.
- `deliver_event_posts_and_records_ok` and `deliver_event_records_error_when_channel_fails`: change to `deliver_event(&store, &ev, RetryPolicy::default(), Utc::now(), None)` (and the `policy` variant likewise adds a trailing `, None`).

- [ ] **Step 4: Thread `smtp` through `scheduler.rs`**

Add the import at the top of `src/scheduler.rs`:

```rust
use crate::config::SmtpConfig;
```

Change `run_scan_loop`'s signature:

```rust
pub async fn run_scan_loop(store: Store, env_default_secs: u64, smtp: Option<SmtpConfig>) {
```

In each of the two delivery spawns (the `scan_once` events loop and the `nag_once` events loop), clone `smtp` into the task and pass it:

```rust
                for ev in events {
                    let store = store.clone();
                    let smtp = smtp.clone();
                    tokio::spawn(async move {
                        deliver_event(&store, &ev, RetryPolicy::default(), Utc::now(), smtp.as_ref())
                            .await;
                    });
                }
```

- [ ] **Step 5: Thread `smtp` through `ping.rs`**

Add imports at the top of `src/ping.rs`:

```rust
use crate::config::{Config, SmtpConfig};
use std::sync::Arc;
```

For each of the five handlers (`success`, `fail`, `start`, `log`, `exitcode`), add a `State(config): State<Arc<Config>>` extractor and pass `config.smtp.clone()` as the new last arg to `apply`. Example for `success` (apply the same shape to all five):

```rust
async fn success(
    State(store): State<Store>,
    State(config): State<Arc<Config>>,
    Path(uuid): Path<String>,
    conn: ClientIp,
    body: Bytes,
) -> Result<StatusCode, AppError> {
    apply(&store, &uuid, PingKind::Success, None, &body, conn, config.smtp.clone()).await
}
```

Change `apply`'s signature to take the new param:

```rust
async fn apply(
    store: &Store,
    uuid: &str,
    kind: PingKind,
    exit_code: Option<i64>,
    body: &[u8],
    conn: ClientIp,
    smtp: Option<SmtpConfig>,
) -> Result<StatusCode, AppError> {
```

At both `spawn_delivery(...)` calls inside `apply`, add `smtp.clone()` as the final argument.

Change `spawn_delivery`'s signature and body:

```rust
fn spawn_delivery(
    store: Store,
    check_id: i64,
    check_name: String,
    project_id: i64,
    event: EventKind,
    now: chrono::DateTime<chrono::Utc>,
    smtp: Option<SmtpConfig>,
) {
    tokio::spawn(async move {
        let ev = NotificationEvent {
            check_id,
            check_name,
            event,
            at: now,
            project_id,
        };
        deliver_event(&store, &ev, RetryPolicy::default(), now, smtp.as_ref()).await;
    });
}
```

- [ ] **Step 6: Update `web.rs` `channel_test`**

In `src/web.rs`, change the `notifier_for(&channel)` call in `channel_test` to:

```rust
    let result = match notifier_for(&channel, state.config.smtp.as_ref()) {
```

- [ ] **Step 7: Update `main.rs`**

In `src/main.rs`, add before the spawn (after `let prune_interval_secs = ...`):

```rust
    let smtp = config.smtp.clone();
```

Change the scan-loop spawn to:

```rust
    tokio::spawn(scheduler::run_scan_loop(
        store.clone(),
        scan_interval_secs,
        smtp,
    ));
```

- [ ] **Step 8: Full suite + fmt + clippy**

Run: `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run`
Expected: compiles; whole SQLite suite green (no behavior change — every arm still ignores `smtp`).

- [ ] **Step 9: Commit**

```bash
git add src/state.rs src/notify.rs src/scheduler.rs src/ping.rs src/web.rs src/main.rs
git commit -m "refactor: thread instance SMTP config to notifier call sites

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Email channel backend (lettre + notifier + kind)

**Files:**
- Modify: `Cargo.toml` (add `lettre`)
- Modify: `src/models.rs` (`ChannelKind::Email`; update round-trip test)
- Modify: `src/notify.rs` (`EmailNotifier`, `build_email`, `notifier_for` Email arm)
- Modify: `src/web.rs` (`ChannelForm.email_to`; `channel_create` Email arm)
- Test: `src/notify.rs`, `src/models.rs`

**Interfaces:**
- Consumes: `SmtpConfig`, `SmtpTls` (Task 1); `smtp` param (Task 2).
- Produces: `ChannelKind::Email`; `EmailNotifier`; `build_email(from, to, ev) -> Result<lettre::Message, NotifyError>`.

- [ ] **Step 1: Add the `lettre` dependency**

In `Cargo.toml` `[dependencies]`, add:

```toml
lettre = { version = "0.11", default-features = false, features = ["tokio1-rustls-tls", "smtp-transport", "builder"] }
```

Run: `cargo build 2>&1 | tail -3` — expected: compiles (fetches lettre 0.11.22).

- [ ] **Step 2: Write the failing tests (models + notify)**

In `src/models.rs`, update `channel_kind_roundtrips`: add `ChannelKind::Email` to the array and change the last line to:

```rust
        assert_eq!(ChannelKind::from_str("email").unwrap(), ChannelKind::Email);
        assert!(ChannelKind::from_str("carrier-pigeon").is_err());
```

In `src/notify.rs` tests, add:

```rust
    #[test]
    fn build_email_sets_headers_and_builds() {
        let ev = NotificationEvent {
            check_id: 0,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
        };
        let msg = build_email("alerts@example.com", "ops@example.com", &ev).unwrap();
        let raw = String::from_utf8(msg.formatted()).unwrap();
        assert!(raw.contains("From: alerts@example.com"), "got: {raw}");
        assert!(raw.contains("To: ops@example.com"), "got: {raw}");
        assert!(raw.contains("Subject:") && raw.contains("pingward"), "got: {raw}");
    }

    #[test]
    fn build_email_rejects_bad_address() {
        let ev = NotificationEvent {
            check_id: 0,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
        };
        assert!(build_email("not-an-address", "ops@example.com", &ev).is_err());
    }

    #[tokio::test]
    async fn email_notifier_errors_when_smtp_unconfigured() {
        let n = EmailNotifier {
            smtp: None,
            to: "ops@example.com".into(),
        };
        let ev = NotificationEvent {
            check_id: 0,
            check_name: "backup".into(),
            event: EventKind::Down,
            at: Utc::now(),
            project_id: 1,
        };
        let err = n.send(&ev).await.unwrap_err();
        assert!(
            err.to_string().contains("instance SMTP not configured"),
            "got: {err}"
        );
    }

    #[test]
    fn notifier_for_email_needs_recipient() {
        assert!(notifier_for(
            &channel_with(ChannelKind::Email, "{\"to\":\"ops@example.com\"}"),
            None
        )
        .is_some());
        assert!(notifier_for(&channel_with(ChannelKind::Email, "{}"), None).is_none());
    }
```

- [ ] **Step 3: Run the new tests to verify they fail**

Run: `cargo nextest run --lib notify::tests::build_email_sets_headers_and_builds`
Expected: FAIL to compile — `build_email`/`EmailNotifier`/`ChannelKind::Email` do not exist.

- [ ] **Step 4: Add `ChannelKind::Email`**

In `src/models.rs`, change the channel-kind enum line to:

```rust
str_enum!(ChannelKind { Webhook => "webhook", Telegram => "telegram", Slack => "slack", Ntfy => "ntfy", Pushover => "pushover", Email => "email" });
```

- [ ] **Step 5: Add lettre imports, `build_email`, and `EmailNotifier` in `src/notify.rs`**

Add imports near the top (after existing `use` lines; `use crate::config::SmtpConfig;` already added in Task 2):

```rust
use crate::config::SmtpTls;
use lettre::message::Message;
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::AsyncSmtpTransport;
use lettre::{AsyncTransport, Tokio1Executor};
```

Add the builder and notifier (place after the other notifier structs, before `notifier_for`):

```rust
/// Build the plain-text email for an event. Pure and panic-free: a malformed
/// address yields a `NotifyError` rather than panicking. Subject reuses
/// `event_title` (control-char sanitized); body is the one-line `event_text`.
fn build_email(from: &str, to: &str, ev: &NotificationEvent) -> Result<Message, NotifyError> {
    Message::builder()
        .from(
            from.parse()
                .map_err(|e| NotifyError(format!("invalid from address: {e}")))?,
        )
        .to(to
            .parse()
            .map_err(|e| NotifyError(format!("invalid recipient address: {e}")))?)
        .subject(event_title(ev))
        .body(event_text(ev))
        .map_err(|e| NotifyError(format!("failed to build email: {e}")))
}

/// Email via the instance SMTP relay. `smtp` is `None` when the relay is not
/// configured — `send` then reports a recorded delivery error rather than
/// silently dropping the alert.
pub struct EmailNotifier {
    smtp: Option<SmtpConfig>,
    to: String,
}

impl Notifier for EmailNotifier {
    fn send<'a>(
        &'a self,
        ev: &'a NotificationEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), NotifyError>> + Send + 'a>> {
        Box::pin(async move {
            let smtp = self
                .smtp
                .as_ref()
                .ok_or_else(|| NotifyError("instance SMTP not configured".into()))?;
            let msg = build_email(&smtp.from, &self.to, ev)?;
            let builder = match smtp.tls {
                SmtpTls::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host)
                    .map_err(|e| NotifyError(format!("smtp setup failed: {e}")))?,
                SmtpTls::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host)
                    .map_err(|e| NotifyError(format!("smtp setup failed: {e}")))?,
                SmtpTls::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp.host),
            };
            let mut builder = builder.port(smtp.port);
            if let (Some(u), Some(p)) = (&smtp.username, &smtp.password) {
                builder = builder.credentials(Credentials::new(u.clone(), p.clone()));
            }
            let transport = builder.build();
            transport
                .send(msg)
                .await
                .map_err(|e| NotifyError(format!("smtp send failed: {e}")))?;
            Ok(())
        })
    }
}
```

- [ ] **Step 6: Add the Email arm to `notifier_for` and use `smtp`**

Rename the param `_smtp` → `smtp`:

```rust
pub fn notifier_for(channel: &Channel, smtp: Option<&SmtpConfig>) -> Option<Box<dyn Notifier>> {
```

Add an Email arm to `match channel.kind` (after the `Pushover` arm):

```rust
        ChannelKind::Email => match cfg_str(&cfg, "to") {
            Some(to) => Some(Box::new(EmailNotifier {
                smtp: smtp.cloned(),
                to,
            })),
            None => missing("to"),
        },
```

- [ ] **Step 7: Add the Email arm to `web.rs` `channel_create` + form field**

Add to `ChannelForm`:

```rust
    #[serde(default)]
    email_to: String,
```

Add an Email arm to `channel_create`'s `match kind` (after the `Pushover` arm):

```rust
        ChannelKind::Email => {
            let to = form.email_to.trim();
            if to.is_empty() {
                return err("an email recipient address is required");
            }
            serde_json::json!({ "to": to }).to_string()
        }
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo nextest run --lib notify && cargo nextest run --lib models::tests::channel_kind_roundtrips`
Expected: PASS.

- [ ] **Step 9: Full suite + fmt + clippy + commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run
git add Cargo.toml Cargo.lock src/models.rs src/notify.rs src/web.rs
git commit -m "feat: add Email (SMTP) notification channel via lettre

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Channel form gating + integration tests

**Files:**
- Modify: `src/web.rs` (`ChannelFormTemplate.smtp_available`; set it in `channel_new` and the `channel_create` error re-render)
- Modify: `templates/channel_form.html` (conditional Email option + recipient input)
- Test: `tests/auth_web.rs` (form gating + create integration)

**Interfaces:**
- Consumes: `Config.smtp` (Task 1), `ChannelKind::Email` + `email_to` (Task 3).
- Produces: Email option shown only when instance SMTP is configured.

- [ ] **Step 1: Write the failing integration tests**

Add a helper and tests to `tests/auth_web.rs`:

```rust
async fn server_with_project_and_smtp() -> (TestServer, Store, i64) {
    use pingward::{app, config::Config, state::AppState, store::Store};
    let pool = pingward::db::connect("sqlite::memory:").await.unwrap();
    pingward::db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let cfg = Config::from_map(|k| match k {
        "PINGWARD_SMTP_HOST" => Some("mail.example.com".into()),
        "PINGWARD_SMTP_FROM" => Some("alerts@example.com".into()),
        _ => None,
    });
    let state = AppState::new(store.clone(), cfg);
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", "admin"), ("password", "pw")])
        .await;
    let pid = store
        .create_project(uid, "p", None, None, chrono::Utc::now())
        .await
        .unwrap();
    (server, store, pid)
}

#[tokio::test]
async fn channel_form_hides_email_without_smtp() {
    let (server, _store, pid) = server_with_project().await;
    let res = server.get(&format!("/projects/{pid}/channels/new")).await;
    res.assert_status_ok();
    assert!(
        !res.text().contains("value=\"email\""),
        "email option must be hidden when SMTP is unconfigured"
    );
}

#[tokio::test]
async fn channel_form_shows_email_with_smtp() {
    let (server, _store, pid) = server_with_project_and_smtp().await;
    let res = server.get(&format!("/projects/{pid}/channels/new")).await;
    res.assert_status_ok();
    assert!(
        res.text().contains("value=\"email\""),
        "email option must appear when SMTP is configured"
    );
}

#[tokio::test]
async fn create_email_channel_stores_recipient() {
    let (server, store, pid) = server_with_project_and_smtp().await;
    let res = server
        .post(&format!("/projects/{pid}/channels"))
        .form(&[("name", "ops"), ("kind", "email"), ("email_to", "ops@example.com")])
        .await;
    res.assert_status(axum::http::StatusCode::SEE_OTHER);
    let channels = store.list_channels_for_project(pid).await.unwrap();
    assert_eq!(channels.len(), 1);
    assert!(channels[0].config_json.contains("ops@example.com"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --test auth_web channel_form_shows_email_with_smtp create_email_channel_stores_recipient channel_form_hides_email_without_smtp`
Expected: FAIL — template has no email option; `channel_form_hides_email_without_smtp` may pass already, but the `_with_smtp` ones fail.

- [ ] **Step 3: Add `smtp_available` to `ChannelFormTemplate` and set it**

Add to `ChannelFormTemplate`:

```rust
    smtp_available: bool,
```

In `channel_new`:

```rust
    Ok(render(&ChannelFormTemplate {
        show_nav: true,
        project_id: pid,
        error: None,
        smtp_available: state.config.smtp.is_some(),
    })?
    .into_response())
```

In `channel_create`, the `err` closure builds a `ChannelFormTemplate` — add `smtp_available: state.config.smtp.is_some(),` to that struct literal too (`state` is captured by the closure).

- [ ] **Step 4: Add the conditional Email option + input to the template**

In `templates/channel_form.html`, inside the `<select>` after the `pushover` option, add:

```html
      {% if smtp_available %}<option value="email">email</option>{% endif %}
```

After the `pushover` `.cfg` div, add:

```html
  {% if smtp_available %}
  <div class="cfg" data-kind="email">
    <label>Recipient email <input name="email_to" type="email"></label>
  </div>
  {% endif %}
```

(The existing `sync()` script toggles `.cfg` divs by `data-kind` generically — no JS change.)

- [ ] **Step 5: Run the integration tests to verify they pass**

Run: `cargo nextest run --test auth_web channel_form_shows_email_with_smtp create_email_channel_stores_recipient channel_form_hides_email_without_smtp`
Expected: PASS.

- [ ] **Step 6: Full suite + fmt + clippy + commit**

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo nextest run
git add src/web.rs templates/channel_form.html tests/auth_web.rs
git commit -m "feat: gate Email channel option on instance SMTP availability

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Final Verification (after all tasks)

- [ ] `cargo fmt --all --check` clean.
- [ ] `cargo clippy --all-targets -- -D warnings` clean.
- [ ] `cargo nextest run` — full SQLite suite green.
- [ ] Live PG round-trip: `TEST_DATABASE_URL=postgres://postgres:postgres@192.168.64.3:5432/postgres cargo nextest run --test pg_store` green (no schema change; confirm the Email kind stores/loads).
- [ ] Manual sanity: with `PINGWARD_SMTP_*` unset, the channel form shows no Email option; with them set, Email appears and Send-test on an Email channel reports either delivery or "instance SMTP not configured".

## Self-Review Notes

- **Spec coverage:** SmtpConfig/env (Task 1), config threading to all 3 call sites + `FromRef` (Task 2), lettre + EmailNotifier + build_email + ChannelKind::Email + notifier_for/channel_create arms (Task 3), form gating + tests (Task 4). All spec sections mapped.
- **Exhaustiveness coupling:** `ChannelKind::Email` (Task 3) forces new arms in `notifier_for` (notify.rs) and `channel_create` (web.rs) in the SAME task so the code compiles; both included in Task 3.
- **Type consistency:** `notifier_for(channel, smtp: Option<&SmtpConfig>)` and `deliver_event(..., smtp: Option<&SmtpConfig>)` fixed in Task 2; `run_scan_loop(..., smtp: Option<SmtpConfig>)` (owned, cloned per spawn); ping `apply`/`spawn_delivery` take owned `Option<SmtpConfig>`. `EmailNotifier { smtp: Option<SmtpConfig>, to: String }` fields match the Task 3 test literals.
- **No schema change:** events remain down/up/reminder; `notifications.event` CHECK untouched.
- **Credential safety:** `build_email`/transport map errors to `NotifyError` without embedding username/password.
