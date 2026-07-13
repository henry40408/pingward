# Email (SMTP) Notification Channel — Design

## Goal

Add an Email notification channel so a check going Down/Up/Reminder can alert
by email. Email is the one `ChannelKind` currently unsupported (a test asserts
`ChannelKind::from_str("email").is_err()`).

## Model: instance-level SMTP (Model A)

pingward acts as an **SMTP client to an operator-provided relay**, never an
MTA. SMTP connection settings (host, port, auth, from, TLS) are configured
**once at the instance level via environment variables** — mirroring how
Healthchecks.io (the closest dead-man's-switch analog), Gatus, and Alertmanager
do it. An Email *channel* stores only a recipient address in its per-channel
`config_json`.

Rationale (decided during brainstorming):
- Matches the self-hosted single-operator deployment reality (one relay).
- Keeps SMTP relay credentials **out of the database** (channel `config_json`
  is stored as plaintext, like Telegram tokens today; a relay password has a
  far larger blast radius).
- Smallest UI surface: the Email channel form is a single recipient field.
- Does not preclude a future per-channel override (additive, not a rewrite).

## Non-Goals (YAGNI)

- Multiple recipients per channel (one `to` per channel; create several
  channels for several recipients — consistent with Telegram `chat_id` /
  Pushover `user`).
- Per-channel SMTP config (Model B).
- HTML email body (plain text only, matching the one-line style of the other
  channels).
- OAuth2 / XOAUTH2 (Gmail app-relay style); username+password AUTH only.
- A proactive "this Email channel is inactive" banner in the channel list
  (possible follow-up; discoverability is covered by the runtime error path).

## Components

### 1. Instance SMTP config (`src/config.rs`)

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpTls { Starttls, Tls, None }

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

`Config` gains `pub smtp: Option<SmtpConfig>`, parsed in `from_map`:

| env var | required | default |
| --- | --- | --- |
| `PINGWARD_SMTP_HOST` | yes (gates `Some`) | — |
| `PINGWARD_SMTP_FROM` | yes (gates `Some`) | — |
| `PINGWARD_SMTP_PORT` | no | `587` |
| `PINGWARD_SMTP_USERNAME` | no | none |
| `PINGWARD_SMTP_PASSWORD` | no | none |
| `PINGWARD_SMTP_TLS` | no | `starttls` |

Rules:
- `smtp` is `Some` only when **both** `HOST` and `FROM` are present and
  non-blank. Otherwise `None` (SMTP unavailable).
- `PINGWARD_SMTP_TLS` parses `starttls` / `tls` / `none` case-insensitively;
  any other value falls back to `starttls`.
- `PINGWARD_SMTP_PORT` that is blank or non-numeric falls back to `587`.
- AUTH is used only when **both** username and password are present; otherwise
  the relay is contacted without authentication (localhost/internal relays).

### 2. Dependency: `lettre`

Add `lettre` with `default-features = false` and features
`["tokio1-rustls-tls", "smtp-transport", "builder"]`. Async transport:
`AsyncSmtpTransport<Tokio1>`. Selected at send time by `SmtpTls`:
- `Tls` → `AsyncSmtpTransport::<Tokio1>::relay(host)` (implicit TLS, default
  port 465 unless overridden).
- `Starttls` → `AsyncSmtpTransport::<Tokio1>::starttls_relay(host)`.
- `None` → `AsyncSmtpTransport::<Tokio1>::builder_dangerous(host)` (plaintext).

Each sets `.port(port)` and, when auth applies, `.credentials(Credentials)`.
The exact lettre version is pinned at implementation time and MUST satisfy the
"published ≥ 7 days ago" dependency rule.

### 3. Channel kind (`src/models.rs`)

Add `Email => "email"` to the `str_enum!(ChannelKind { … })`. The channel's
`config_json` is `{"to": "<address>"}`. Events remain `down`/`up`/`reminder`
— **no schema/migration change** (the `notifications.event` CHECK constraint is
untouched). The existing `channel_kind_roundtrips` test's
`assert!(ChannelKind::from_str("email").is_err())` MUST be updated to expect the
`Email` variant to round-trip.

### 4. Notifier (`src/notify.rs`)

`EmailNotifier`:

```rust
pub struct EmailNotifier {
    smtp: Option<SmtpConfig>, // instance config; None => unconfigured
    to: String,               // recipient, from channel config_json
}
```

- `event_title(ev)` is reused for the **Subject** (`pingward: <name> <event>`,
  already control-char sanitized).
- Body is the plain-text `event_text(ev)` one-liner (Test/Down/Up/Reminder).
- Extract a pure builder for testability:
  `fn build_email(from: &str, to: &str, ev: &NotificationEvent) -> Result<lettre::Message, NotifyError>`
  — sets From/To/Subject/plain body; a malformed address yields
  `NotifyError` (never panics).
- `send()`:
  - `smtp` is `None` → `Err(NotifyError("instance SMTP not configured"))`.
  - else build the message, build the transport per `SmtpTls`, send, and map a
    lettre error to a `NotifyError` **without leaking credentials** (report a
    classification / lettre's message, never the password).

`notifier_for(channel, smtp: Option<&SmtpConfig>)`:
- Email arm: read `to` from `config_json`; missing/blank → `None` (skip, like
  any other channel with incomplete per-channel config). Valid `to` → always
  `Some(EmailNotifier { smtp: smtp.cloned(), to })` — even when `smtp` is
  `None`, so the unconfigured case surfaces as a recorded delivery error rather
  than a silent skip.

`deliver_event(store, ev, policy, now, smtp: Option<&SmtpConfig>)` — threads
`smtp` through to `notifier_for`. All other channel arms ignore `smtp`.

### 5. Config threading (call sites)

`notifier_for` / `deliver_event` currently receive no `Config`. Thread the
instance SMTP config to the three call sites:

- **`src/web.rs`** `channel_test`: `state.config.smtp.as_ref()`.
- **`src/ping.rs`** ingest handler: from `AppState` → `state.config.smtp`.
- **`src/scheduler.rs`** `run_scan_loop(store, env_default_secs, smtp)`:
  new `smtp: Option<SmtpConfig>` param, passed to each `deliver_event` call.
  **`src/main.rs`** clones `config.smtp` and passes it to
  `run_scan_loop` **before** `config` is moved into `AppState`.

### 6. Web UI (`src/web.rs`, `templates/channel_form.html`)

- `ChannelForm` gains `#[serde(default)] email_to: String`.
- `channel_create` Email arm: trim `email_to`; empty → the existing inline
  error ("an email recipient is required"); else `config_json = {"to": to}`.
- **Hide when unavailable:** `ChannelFormTemplate` gains
  `smtp_available: bool` (from `state.config.smtp.is_some()`). `channel_new`
  and the error re-render paths set it. In `channel_form.html`, the Email
  `<option>` and its recipient input render only when `smtp_available` is true.
- The per-channel **Send test** button (from #16) works unchanged: it already
  calls `notifier_for(&channel, …)`; the Email arm returns an `EmailNotifier`
  whose `send()` either delivers or reports "instance SMTP not configured".

### 7. Unconfigured / dangling behavior (decided)

- **At create time:** when `Config.smtp` is `None`, the channel form omits the
  Email option, so a user cannot create an Email channel with no backend.
- **When env is removed after a channel exists:** the channel remains in the
  DB; delivery builds the `EmailNotifier` with `smtp: None`, whose `send()`
  returns `Err("instance SMTP not configured")`. `deliver_event` records this
  as a failed notification, so it appears in the check's notification history
  (Channel = the email channel's name, Error = "instance SMTP not configured")
  and in the Send-test banner. No crash; other channels are unaffected.

## Data Flow

```
Operator sets PINGWARD_SMTP_* → Config.smtp = Some(SmtpConfig)
User adds Email channel (form shows Email only when smtp available) → config_json {"to": …}
Check transitions (Down/Up/Reminder) → deliver_event(store, ev, policy, now, smtp)
  → notifier_for(email_channel, smtp) → EmailNotifier
    → smtp Some: build_email → AsyncSmtpTransport(per TLS) → send → record ok/err
    → smtp None: send() Err("instance SMTP not configured") → record err (visible in history)
Send-test button → same EmailNotifier path → banner
```

## Error Handling

- Malformed recipient / from address → `NotifyError` from `build_email`
  (recorded as a delivery error; never panics).
- SMTP transport failure (connect/auth/timeout) → `NotifyError` that does **not**
  include the password (reuse the redaction discipline from #18).
- Instance SMTP unconfigured at send time → `NotifyError("instance SMTP not
  configured")`, recorded.
- Channel `to` missing → `notifier_for` returns `None` (skipped, not recorded)
  — the same treatment as any other channel with incomplete config.

## Testing

- **`src/config.rs`:** SMTP env parsing — all-present → `Some`; missing HOST or
  FROM → `None`; `PINGWARD_SMTP_TLS` maps `starttls`/`tls`/`none` (and unknown →
  `starttls`); blank/non-numeric port → 587; auth active only when both
  username and password set.
- **`src/notify.rs`:**
  - `build_email` sets a correct From/To/Subject and plain body; a malformed
    address returns `Err`, not a panic.
  - `notifier_for` Email arm: valid `to` → `Some`; missing `to` → `None`.
  - `EmailNotifier::send()` with `smtp: None` → `Err("instance SMTP not
    configured")` (no network).
  - `ChannelKind` round-trip now includes `Email`; update the
    `from_str("email").is_err()` assertion.
- **Send path over the network:** primarily covered by the pure `build_email`
  test and the unconfigured-error path. A real end-to-end SMTP send test
  (against an in-process mock listener or lettre's stub) is **best-effort** —
  included if a mock is straightforward, otherwise omitted with a note.
- **Regression:** existing SQLite suite green, live PG `pg_store` green,
  `cargo fmt` / `cargo clippy --all-targets -D warnings` clean.

## Files Touched

- `Cargo.toml` — add `lettre`.
- `src/config.rs` — `SmtpTls`, `SmtpConfig`, `Config.smtp`, env parsing + tests.
- `src/models.rs` — `ChannelKind::Email`; update round-trip test.
- `src/notify.rs` — `EmailNotifier`, `build_email`, `notifier_for`/`deliver_event`
  signatures gain `smtp`; tests.
- `src/scheduler.rs` — `run_scan_loop` gains `smtp`; `deliver_event` calls.
- `src/ping.rs` — pass `state.config.smtp` to `deliver_event`.
- `src/web.rs` — `channel_test` passes smtp; `ChannelForm.email_to`;
  `channel_create` Email arm; `ChannelFormTemplate.smtp_available`.
- `src/main.rs` — thread `config.smtp` into `run_scan_loop`.
- `templates/channel_form.html` — conditional Email option + recipient input.
