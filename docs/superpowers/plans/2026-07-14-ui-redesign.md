# Pingward UI Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Pingward's bare inline-CSS UI with a coherent "Console" design system — dual light/dark themes, a runtime-headroom heartbeat visualization, derived `late`/`new` states, and surfacing of already-captured ping body/source_ip/log data — without leaving the server-rendered stack.

**Architecture:** Keep `axum` + `askama`. Add one embedded static-asset route serving a single `app.css` and two self-hosted `woff2` fonts (no new crates, no runtime CDN). Add a read-only `view` module that derives display state and heartbeat bars from existing models. Every template is restyled against the shared stylesheet; a sprinkle of vanilla JS handles theme toggle, ping-body expand, and URL copy.

**Tech Stack:** Rust, axum 0.8, askama 0.16 (HTML-escaped by default), chrono, vanilla JS/CSS. Tests: `cargo nextest run`, `axum-test`.

**Design source of truth (approved mockups, on disk, gitignored):**
- `.superpowers/brainstorm/7729-1784008741/content/console-v2.html` — dashboard (tokens, top bar, tiles, groups, check rows, heartbeat, legend, badges).
- `.superpowers/brainstorm/7729-1784008741/content/check-detail.html` — detail page, tables, expandable output, form controls, buttons, checkboxes.
- Spec: `docs/superpowers/specs/2026-07-14-ui-redesign-design.md`.

## Global Constraints

- **Toolchain/commands:** run from repo root `/Users/henry/Develop/claude/pingward`; `pwd` before build/test/git. Tests via `cargo nextest run` (never `cargo test`). `cargo fmt` before every commit. Commits GPG-signed (never `--no-gpg-sign`). Stage files explicitly by name (never `git add -A`/`.`).
- **No new dependencies.** Serve CSS/fonts with `include_str!`/`include_bytes!` + plain axum handlers. (Any genuinely-needed crate must clear the 7-day-old rule and user sign-off first.)
- **No schema migrations.** `late` and per-ping `duration` are derived at read time only.
- **No runtime CDN.** Fonts are self-hosted `woff2`; CSS has `system-ui` / `ui-monospace` fallbacks.
- **Preserve behaviour + test substrings.** Existing `axum-test` cases assert on `res.text().contains(...)` (e.g. `value="email"`, `Test notification sent`, channel names, `status 500`). Restyled templates MUST keep these visible strings and all existing form field `name=`/`value=` attributes, routes, and POST actions.
- **Copy voice:** end-user language, sentence case, active verbs (spec §Writing). e.g. button says `Save changes`, `Acknowledge`, `New check`.
- **Design tokens are verbatim** from spec §4 (both themes).

---

## File Structure

- Create `src/assets.rs` — static-asset router (`/assets/app.css`, `/assets/fonts/*.woff2`).
- Create `assets/app.css` — the whole design system (tokens + components).
- Create `assets/fonts/inter-{400,500,600,700}.woff2`, `assets/fonts/ibm-plex-mono-{400,500,600}.woff2` — vendored, OFL.
- Create `src/view.rs` — read-only view helpers: `DisplayStatus`, `display_status()`, `run_durations()`, `heartbeat()`, `fmt_secs()`, `fmt_relative()`.
- Modify `src/lib.rs` — declare modules, merge `assets::routes()`.
- Modify `src/web.rs` — enrich view-models (dashboard groups + counts, check rows with heartbeat, detail ping rows with body/source/duration).
- Modify all templates under `templates/` — restyle against `app.css`.
- Modify/extend tests under `tests/` and add `#[cfg(test)]` unit tests in `src/view.rs`.

---

### Task 1: Static-asset route + design-system stylesheet + new shell

Delivers the shared stylesheet and a restyled `base.html` shell (top bar, tokens, theme toggle) that every page inherits. Fonts fall back to system stacks until Task 2 vendors woff2. Page bodies still use their old markup — functional, half-styled — until later tasks.

**Files:**
- Create: `src/assets.rs`, `assets/app.css`
- Modify: `src/lib.rs`, `templates/base.html`
- Test: `tests/assets.rs`

**Interfaces:**
- Produces: `pingward::assets::routes() -> axum::Router<AppState>` serving `GET /assets/app.css` (`text/css`) and `GET /assets/fonts/{file}` (`font/woff2`).

- [ ] **Step 1: Write the failing test** — `tests/assets.rs`:

```rust
use axum_test::TestServer;

mod common; // if a shared helper exists; otherwise inline app construction like other tests

#[tokio::test]
async fn serves_stylesheet() {
    let server = common::server().await; // mirror the setup used in tests/auth_web.rs
    let res = server.get("/assets/app.css").await;
    res.assert_status_ok();
    assert_eq!(res.header("content-type"), "text/css; charset=utf-8");
    assert!(res.text().contains("--brand"), "tokens missing from css");
}
```

> If `tests/` has no `common` module, replicate the exact app/server construction already used at the top of `tests/auth_web.rs` (build `AppState`, `pingward::app(state)`, `TestServer::new`). Match that pattern rather than inventing one.

- [ ] **Step 2: Run it, verify it fails** — `cargo nextest run --test assets` → FAIL (route 404 / module missing).

- [ ] **Step 3: Create `src/assets.rs`:**

```rust
use crate::state::AppState;
use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

const APP_CSS: &str = include_str!("../assets/app.css");

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/assets/app.css", get(app_css))
        .route("/assets/fonts/{file}", get(font))
}

async fn app_css() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        APP_CSS,
    )
}

async fn font(Path(file): Path<String>) -> impl IntoResponse {
    // Vendored in Task 2. Until then, unknown files 404 cleanly.
    let bytes: Option<&'static [u8]> = match file.as_str() {
        _ => None,
    };
    match bytes {
        Some(b) => (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            b,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
```

- [ ] **Step 4: Register in `src/lib.rs`** — add `pub mod assets;` (alongside the other `pub mod`s) and add `.merge(assets::routes())` inside `app()`:

```rust
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .merge(web::routes())
        .merge(ping::routes())
        .merge(assets::routes())
        .with_state(state)
}
```

- [ ] **Step 5: Create `assets/app.css`** — the full stylesheet. Port it from the two mockup files (design source of truth). Concretely:
  1. Copy the `:root` (dark) and `:root[data-theme="light"]` token blocks **verbatim from spec §4** (add `--new` in both). These override the mockups' inline `[data-theme]` selectors — use `:root[data-theme="light"]`, not the demo's `[data-theme="light"]`.
  2. Copy every component rule from `console-v2.html` (`body`, `.wrap`, `header.bar`+children, `.brand`, `nav.links`, `.iconbtn`, `.ghost`, `.phead`, `.btn-primary`, `.tiles`/`.tile`, `.group`/`.gh`, `.list`, `.check`, `.status-dot` + ripple, `.cmeta`, `.cwhen`, `.spark` + `i.hot/.bad/.none/.pausedbar`, `.badge` + status variants, `.legend`) and from `check-detail.html` (`.crumb`, `.chead`, `.actions`, `.btn` variants, `.card`/`.ch`/`.cb`, `.urlrow`/`.copy`, `.beat`, `.chk` checkbox, `table`/`thead`/`tbody`, `.pill` variants incl. `.log`, `.caret`, `tr.exp`, `.out` variants, `.field` form controls, `.two`, `h3.sec`).
  3. Add `@media (prefers-reduced-motion: reduce){ *{animation:none!important;transition:none!important} }`.
  4. `@font-face` blocks referencing `/assets/fonts/*.woff2` with `font-display:swap` (files arrive in Task 2; system fallback covers the gap).
  5. Keep the mockups' responsive `@media` rules.

- [ ] **Step 6: Rewrite `templates/base.html`** to the shared shell (keep `{% block body %}` and `show_nav` gating; keep the literal nav link text `Dashboard`/`Settings` and the `Log out` button so existing tests/paths hold):

```html
<!doctype html>
<html lang="en" data-theme="dark">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>pingward</title>
  <link rel="stylesheet" href="/assets/app.css">
  <script>
    (function(){try{var t=localStorage.getItem('pw-theme')||(matchMedia('(prefers-color-scheme: light)').matches?'light':'dark');document.documentElement.setAttribute('data-theme',t);}catch(e){}})();
  </script>
</head>
<body>
  {% if show_nav %}
  <header class="bar"><div class="inner">
    <a class="brand" href="/"><span class="glyph">▚</span>pingward</a>
    <nav class="links"><a href="/">Dashboard</a><a href="/settings">Settings</a></nav>
    <div class="right">
      <button class="iconbtn" id="pw-theme-toggle" title="Toggle theme" aria-label="Toggle theme">◐</button>
      <form class="inline" method="post" action="/logout"><button class="ghost" type="submit">Log out</button></form>
    </div>
  </div></header>
  {% endif %}
  <main class="wrap">{% block body %}{% endblock %}</main>
  <script>
    (function(){var b=document.getElementById('pw-theme-toggle');if(!b)return;b.addEventListener('click',function(){var r=document.documentElement,n=r.getAttribute('data-theme')==='dark'?'light':'dark';r.setAttribute('data-theme',n);try{localStorage.setItem('pw-theme',n);}catch(e){}});})();
  </script>
</body>
</html>
```

- [ ] **Step 7: Run tests** — `cargo fmt && cargo nextest run` → new assets test PASSES; existing suite still green (bodies render inside new shell).

- [ ] **Step 8: Commit** — `git add src/assets.rs assets/app.css src/lib.rs templates/base.html tests/assets.rs && git commit -m "feat(ui): add design-system stylesheet, asset route, and new shell"`

---

### Task 2: Vendor self-hosted fonts

**Files:**
- Create: `assets/fonts/inter-{400,500,600,700}.woff2`, `assets/fonts/ibm-plex-mono-{400,500,600}.woff2`
- Modify: `src/assets.rs` (wire `include_bytes!`)
- Test: extend `tests/assets.rs`

- [ ] **Step 1: Add the failing test** to `tests/assets.rs`:

```rust
#[tokio::test]
async fn serves_a_font() {
    let server = common::server().await;
    let res = server.get("/assets/fonts/inter-400.woff2").await;
    res.assert_status_ok();
    assert_eq!(res.header("content-type"), "font/woff2");
}
```

- [ ] **Step 2: Run it, verify it fails** — `cargo nextest run --test assets` → FAIL (404).

- [ ] **Step 3: Vendor the woff2 files** (OFL, latin subset). Use google-webfonts-helper (no account needed), e.g.:

```bash
cd assets/fonts
for w in 400 500 600 700; do curl -fsSL "https://gwfh.mranftl.com/api/fonts/inter?download=zip&subsets=latin&variants=$w&formats=woff2" -o i$w.zip && unzip -o -j i$w.zip '*.woff2' && rm i$w.zip; done
# then rename downloaded files → inter-400.woff2 … inter-700.woff2
for w in 400 500 600; do curl -fsSL "https://gwfh.mranftl.com/api/fonts/ibm-plex-mono?download=zip&subsets=latin&variants=$w&formats=woff2" -o p$w.zip && unzip -o -j p$w.zip '*.woff2' && rm p$w.zip; done
# rename → ibm-plex-mono-400.woff2 … ibm-plex-mono-600.woff2
```

If the download host is unreachable in this environment, stop and tell the user: "fonts need vendoring — provide the 7 woff2 files or approve system-stack fallback"; the system-stack fallback from Task 1 keeps the app fully functional meanwhile.

- [ ] **Step 4: Wire them in `src/assets.rs`** — replace the `match file` body:

```rust
    let bytes: Option<&'static [u8]> = match file.as_str() {
        "inter-400.woff2" => Some(include_bytes!("../assets/fonts/inter-400.woff2")),
        "inter-500.woff2" => Some(include_bytes!("../assets/fonts/inter-500.woff2")),
        "inter-600.woff2" => Some(include_bytes!("../assets/fonts/inter-600.woff2")),
        "inter-700.woff2" => Some(include_bytes!("../assets/fonts/inter-700.woff2")),
        "ibm-plex-mono-400.woff2" => Some(include_bytes!("../assets/fonts/ibm-plex-mono-400.woff2")),
        "ibm-plex-mono-500.woff2" => Some(include_bytes!("../assets/fonts/ibm-plex-mono-500.woff2")),
        "ibm-plex-mono-600.woff2" => Some(include_bytes!("../assets/fonts/ibm-plex-mono-600.woff2")),
        _ => None,
    };
```

- [ ] **Step 5: Confirm `@font-face` in `assets/app.css`** references these exact filenames/weights.

- [ ] **Step 6: Run tests** — `cargo fmt && cargo nextest run --test assets` → PASS.

- [ ] **Step 7: Commit** — `git add assets/fonts src/assets.rs assets/app.css && git commit -m "feat(ui): self-host Inter and IBM Plex Mono woff2"`

---

### Task 3: View helpers — derived status, durations, heartbeat, formatting

Pure functions with unit tests; the crux of the redesign's data logic.

**Files:**
- Create: `src/view.rs`
- Modify: `src/lib.rs` (`pub mod view;`)
- Test: inline `#[cfg(test)]` in `src/view.rs`

**Interfaces:**
- Produces:
  - `enum DisplayStatus { New, Up, Late, Down, Paused }` with `fn as_str(self) -> &'static str`.
  - `fn display_status(check: &Check, now: DateTime<Utc>) -> DisplayStatus`
  - `fn run_durations(pings: &[Ping]) -> HashMap<i64, i64>` (finish ping id → seconds)
  - `struct Bar { height: u32, class: &'static str, title: String }`
  - `fn heartbeat(pings: &[Ping], max_runtime_secs: Option<i64>, paused: bool, n: usize) -> Vec<Bar>`
  - `fn fmt_secs(secs: i64) -> String` (e.g. `4m 02s`, `50s`, `1h 03m`)
  - `fn fmt_relative(then: DateTime<Utc>, now: DateTime<Utc>) -> String` (e.g. `3m ago`)

- [ ] **Step 1: Write failing tests** — bottom of `src/view.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Check, CheckStatus, Ping, PingKind, ScheduleKind};
    use chrono::{Duration, TimeZone, Utc};

    fn base_check() -> Check {
        Check {
            id: 1, project_id: 1, name: "c".into(), ping_uuid: "u".into(),
            schedule_kind: ScheduleKind::Period, period_secs: Some(3600),
            grace_secs: 300, cron_expr: None, timezone: "UTC".into(),
            status: CheckStatus::Up, last_ping_at: None, last_start_at: None,
            next_due_at: None, scan_interval_secs: None, max_runtime_secs: None,
            nag_interval_secs: None, last_alert_at: None, acknowledged: false,
            created_at: Utc::now(),
        }
    }
    fn ping(id: i64, kind: PingKind, at: chrono::DateTime<Utc>) -> Ping {
        Ping { id, check_id: 1, kind, exit_code: None, body: String::new(), source_ip: None, created_at: at }
    }

    #[test]
    fn up_in_grace_window_is_late() {
        let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let mut c = base_check();
        c.status = CheckStatus::Up;
        c.next_due_at = Some(now + Duration::seconds(120)); // due in 2m, grace 300 → expected was 3m ago
        assert_eq!(display_status(&c, now), DisplayStatus::Late);
    }

    #[test]
    fn up_before_expected_is_up() {
        let now = Utc.with_ymd_and_hms(2026, 7, 14, 12, 0, 0).unwrap();
        let mut c = base_check();
        c.next_due_at = Some(now + Duration::seconds(3000)); // expected well in the future
        assert_eq!(display_status(&c, now), DisplayStatus::Up);
    }

    #[test]
    fn stored_states_pass_through() {
        let now = Utc::now();
        let mut c = base_check();
        for (s, d) in [(CheckStatus::New, DisplayStatus::New), (CheckStatus::Down, DisplayStatus::Down), (CheckStatus::Paused, DisplayStatus::Paused)] {
            c.status = s;
            assert_eq!(display_status(&c, now), d);
        }
    }

    #[test]
    fn duration_pairs_start_with_next_finish() {
        let t0 = Utc.with_ymd_and_hms(2026, 7, 14, 8, 0, 0).unwrap();
        let pings = vec![
            ping(1, PingKind::Start, t0),
            ping(2, PingKind::Success, t0 + Duration::seconds(242)),
        ];
        let d = run_durations(&pings);
        assert_eq!(d.get(&2), Some(&242));
    }

    #[test]
    fn heartbeat_no_duration_is_hollow() {
        let t0 = Utc.with_ymd_and_hms(2026, 7, 14, 8, 0, 0).unwrap();
        let pings = vec![ping(1, PingKind::Success, t0), ping(2, PingKind::Success, t0 + Duration::seconds(60))];
        let bars = heartbeat(&pings, None, false, 6);
        assert!(bars.iter().all(|b| b.class == "none"));
    }

    #[test]
    fn heartbeat_hot_when_over_80pct_of_max_runtime() {
        let t0 = Utc.with_ymd_and_hms(2026, 7, 14, 8, 0, 0).unwrap();
        let pings = vec![
            ping(1, PingKind::Start, t0),
            ping(2, PingKind::Success, t0 + Duration::seconds(90)), // 90/100 = 90%
        ];
        let bars = heartbeat(&pings, Some(100), false, 6);
        assert_eq!(bars.last().unwrap().class, "hot");
    }

    #[test]
    fn heartbeat_paused_is_flatline() {
        let bars = heartbeat(&[], None, true, 6);
        assert_eq!(bars.len(), 6);
        assert!(bars.iter().all(|b| b.class == "pausedbar"));
    }
}
```

- [ ] **Step 2: Run tests, verify they fail** — `cargo nextest run view` → FAIL (module missing).

- [ ] **Step 3: Implement `src/view.rs`:**

```rust
use crate::models::{Check, CheckStatus, Ping, PingKind};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayStatus { New, Up, Late, Down, Paused }

impl DisplayStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            DisplayStatus::New => "new",
            DisplayStatus::Up => "up",
            DisplayStatus::Late => "late",
            DisplayStatus::Down => "down",
            DisplayStatus::Paused => "paused",
        }
    }
}

/// `next_due_at` already includes grace, so `next_due_at - grace` is the expected
/// run time. A stored-Up check inside `(expected, due]` is "running late".
pub fn display_status(check: &Check, now: DateTime<Utc>) -> DisplayStatus {
    match check.status {
        CheckStatus::New => DisplayStatus::New,
        CheckStatus::Down => DisplayStatus::Down,
        CheckStatus::Paused => DisplayStatus::Paused,
        CheckStatus::Up => {
            if let Some(due) = check.next_due_at {
                let expected = due - Duration::seconds(check.grace_secs);
                if now > expected && now <= due {
                    return DisplayStatus::Late;
                }
            }
            DisplayStatus::Up
        }
    }
}

fn is_finish(k: PingKind) -> bool { matches!(k, PingKind::Success | PingKind::Fail) }

/// Pair each finish (success/fail) ping with the most recent preceding `start`.
/// Input may be newest- or oldest-first; normalized to chronological internally.
pub fn run_durations(pings: &[Ping]) -> HashMap<i64, i64> {
    let mut ordered: Vec<&Ping> = pings.iter().collect();
    ordered.sort_by_key(|p| (p.created_at, p.id));
    let mut out = HashMap::new();
    let mut pending_start: Option<DateTime<Utc>> = None;
    for p in ordered {
        match p.kind {
            PingKind::Start => pending_start = Some(p.created_at),
            k if is_finish(k) => {
                if let Some(s) = pending_start.take() {
                    let secs = (p.created_at - s).num_seconds();
                    if secs >= 0 { out.insert(p.id, secs); }
                }
            }
            _ => {} // log / exitcode-as-recorded: ignore
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Bar { pub height: u32, pub class: &'static str, pub title: String }

const MAX_H: u32 = 26;
const MIN_H: u32 = 5;
const NONE_H: u32 = 16;
const HOT_FRACTION: f64 = 0.80;

/// Build the heartbeat strip: the last `n` runs (success/fail pings), height by
/// fraction of runtime budget used, colour by outcome. See spec §7.
pub fn heartbeat(pings: &[Ping], max_runtime_secs: Option<i64>, paused: bool, n: usize) -> Vec<Bar> {
    if paused {
        return (0..n).map(|_| Bar { height: MIN_H, class: "pausedbar", title: "paused".into() }).collect();
    }
    let durations = run_durations(pings);
    // chronological runs = finish pings, oldest→newest, keep last n
    let mut runs: Vec<&Ping> = pings.iter().filter(|p| is_finish(p.kind)).collect();
    runs.sort_by_key(|p| (p.created_at, p.id));
    let start = runs.len().saturating_sub(n);
    let runs = &runs[start..];

    let measured: Vec<i64> = runs.iter().filter_map(|p| durations.get(&p.id).copied()).collect();
    // Ceiling: explicit max_runtime, else window max — but the window fallback
    // needs >= 2 measured durations to be meaningful.
    let ceiling: Option<i64> = match max_runtime_secs {
        Some(m) if m > 0 => Some(m),
        _ => if measured.len() >= 2 { measured.iter().copied().max() } else { None },
    };

    runs.iter().map(|p| {
        let dur = durations.get(&p.id).copied();
        let failed = p.kind == PingKind::Fail;
        match (dur, ceiling) {
            (Some(d), Some(c)) if c > 0 => {
                let frac = (d as f64 / c as f64).clamp(0.0, 1.0);
                let h = ((MAX_H as f64) * frac).round() as u32;
                let height = h.clamp(MIN_H, MAX_H);
                let class = if failed { "bad" }
                    else if matches!(max_runtime_secs, Some(m) if m > 0 && (d as f64) >= HOT_FRACTION * m as f64) { "hot" }
                    else { "" };
                Bar { height, class, title: format!("{} / {}", fmt_secs(d), fmt_secs(c)) }
            }
            _ => {
                let class = if failed { "bad" } else { "none" };
                let height = if failed { MAX_H } else { NONE_H };
                let title = if failed { "failed".into() } else { "duration unknown".into() };
                Bar { height, class, title }
            }
        }
    }).collect()
}

pub fn fmt_secs(secs: i64) -> String {
    let s = secs.max(0);
    if s < 60 { format!("{s}s") }
    else if s < 3600 { format!("{}m {:02}s", s / 60, s % 60) }
    else { format!("{}h {:02}m", s / 3600, (s % 3600) / 60) }
}

pub fn fmt_relative(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let s = (now - then).num_seconds().max(0);
    if s < 60 { format!("{s}s ago") }
    else if s < 3600 { format!("{}m ago", s / 60) }
    else if s < 86400 { format!("{}h ago", s / 3600) }
    else { format!("{}d ago", s / 86400) }
}
```

- [ ] **Step 4: Declare module** — add `pub mod view;` to `src/lib.rs`.

- [ ] **Step 5: Run tests, verify pass** — `cargo fmt && cargo nextest run view` → all PASS.

- [ ] **Step 6: Commit** — `git add src/view.rs src/lib.rs && git commit -m "feat(ui): add view helpers for status, durations, heartbeat"`

---

### Task 4: Dashboard — summary tiles, project groups, heartbeat rows

**Files:**
- Modify: `src/web.rs` (`DashboardTemplate`/`ProjectRow` view-model), `templates/dashboard.html`
- Test: `tests/dashboard_view.rs`

**Interfaces:**
- Consumes: `view::{display_status, heartbeat, fmt_relative, Bar, DisplayStatus}`, `store::list_recent_pings`.
- Produces view-model rows the template renders (see below).

- [ ] **Step 1: Write the failing test** — `tests/dashboard_view.rs`: sign in (reuse the setup + a helper that creates a project and a check, mirroring `tests/auth_web.rs`), GET `/`, assert the new structure is present:

```rust
let res = server.get("/").await;
res.assert_status_ok();
let body = res.text();
assert!(body.contains("class=\"tiles\""), "summary tiles missing");
assert!(body.contains("class=\"badge"), "status badge missing");
```

- [ ] **Step 2: Run it, verify it fails** — `cargo nextest run --test dashboard_view` → FAIL (old markup).

- [ ] **Step 3: Enrich the view-model in `src/web.rs`.** Replace `ProjectRow`/`DashboardTemplate` with display rows. Add:

```rust
struct CheckRow {
    id: i64,
    name: String,
    status: &'static str,          // view::DisplayStatus::as_str()
    schedule: String,              // e.g. "every 1h · 10m grace" or the cron expr
    last: String,                  // fmt_relative or "—"
    bars: Vec<crate::view::Bar>,
}
struct ProjectGroup { id: i64, name: String, count: usize, checks: Vec<CheckRow> }

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    show_nav: bool,
    total: usize, up: usize, late: usize, down: usize,
    groups: Vec<ProjectGroup>,
}
```

In `dashboard()`, build it (dashboard heartbeat window `n = 6`):

```rust
let now = Utc::now();
let (mut total, mut up, mut late, mut down) = (0usize, 0, 0, 0);
let mut groups = Vec::new();
for project in state.store.list_projects_for_user(user.id).await? {
    let checks = state.store.list_checks_for_project(project.id).await?;
    let mut rows = Vec::with_capacity(checks.len());
    for c in &checks {
        let ds = crate::view::display_status(c, now);
        total += 1;
        match ds {
            crate::view::DisplayStatus::Up => up += 1,
            crate::view::DisplayStatus::Late => late += 1,
            crate::view::DisplayStatus::Down => down += 1,
            _ => {}
        }
        let pings = state.store.list_recent_pings(c.id, 40).await?;
        let bars = crate::view::heartbeat(&pings, c.max_runtime_secs, c.status == CheckStatus::Paused, 6);
        rows.push(CheckRow {
            id: c.id, name: c.name.clone(), status: ds.as_str(),
            schedule: schedule_label(c),
            last: c.last_ping_at.map(|t| crate::view::fmt_relative(t, now)).unwrap_or_else(|| "—".into()),
            bars,
        });
    }
    groups.push(ProjectGroup { id: project.id, name: project.name, count: checks.len(), checks: rows });
}
```

Add a shared helper (also used by the check page):

```rust
fn schedule_label(c: &Check) -> String {
    match c.schedule_kind {
        ScheduleKind::Period => match c.period_secs {
            Some(s) => format!("every {} · {} grace", crate::view::fmt_secs(s), crate::view::fmt_secs(c.grace_secs)),
            None => format!("{} grace", crate::view::fmt_secs(c.grace_secs)),
        },
        ScheduleKind::Cron => c.cron_expr.clone().unwrap_or_default(),
    }
}
```

- [ ] **Step 4: Rewrite `templates/dashboard.html`** using the `console-v2.html` structure. Page header + `New project` primary link, the four tiles bound to `total/up/late/down`, then per group a `.group` header (`{{ g.name }}`, `{{ g.count }} checks`, `Manage →` to `/projects/{{ g.id }}`) and a `.list` of `.check` rows. Each row (link to `/checks/{{ c.id }}`):

```html
<div class="check" onclick="location='/checks/{{ c.id }}'">
  <span class="status-dot {{ c.status }}"></span>
  <div class="cmeta"><div class="nm">{{ c.name }}</div><div class="sc">{{ c.schedule }}</div></div>
  <div class="spark">{% for b in c.bars %}<i class="{{ b.class }}" style="height:{{ b.height }}px" title="{{ b.title }}"></i>{% endfor %}</div>
  <div class="cwhen">{{ c.last }}</div>
  <span class="badge {{ c.status }}">{{ c.status }}</span>
</div>
```

Include the heartbeat `.legend` block from the mockup once, below the groups. Keep an empty-state message when `groups` is empty ("No projects yet. Create one to start watching a job.").

- [ ] **Step 5: Run tests** — `cargo fmt && cargo nextest run` → dashboard test PASS; full suite green.

- [ ] **Step 6: Commit** — `git add src/web.rs templates/dashboard.html tests/dashboard_view.rs && git commit -m "feat(ui): redesign dashboard with tiles, groups, heartbeat rows"`

---

### Task 5: Check detail — header, actions, ping URL, big heartbeat, channels, pings-with-body, notifications

**Files:**
- Modify: `src/web.rs` (`CheckTemplate` + `check_show`), `templates/check.html`
- Test: `tests/check_detail.rs`

**Interfaces:**
- Consumes: `view::{display_status, heartbeat, run_durations, fmt_secs, fmt_relative}`, `store::list_recent_pings` (already returns `Ping` with `body`, `source_ip`).

- [ ] **Step 1: Write failing test** — `tests/check_detail.rs`: create a check, POST a `fail` ping with a body (`server.post("/ping/{uuid}/fail").text("boom trace")`), GET `/checks/{id}`, assert:

```rust
let body = res.text();
assert!(body.contains("class=\"beat\""), "heartbeat missing");
assert!(body.contains("boom trace"), "captured ping body not surfaced");
assert!(body.contains("Source"), "source column missing");
```

- [ ] **Step 2: Run it, verify it fails** — `cargo nextest run --test check_detail` → FAIL.

- [ ] **Step 3: Enrich `src/web.rs`.** Extend `PingRow` and `CheckTemplate`:

```rust
struct PingRow {
    time: String,          // HH:MM:SS (or fmt_relative — pick HH:MM:SS to match mockup)
    kind: &'static str,    // "success"|"fail"|"start"|"log"
    exit: String,          // "exit 0" | "—"
    duration: String,      // fmt_secs | "—"
    source: String,        // source_ip | "—"
    body: String,          // may be empty → no toggle
}

#[derive(Template)]
#[template(path = "check.html")]
struct CheckTemplate {
    show_nav: bool,
    check: Check,
    status: &'static str,      // display_status
    since: String,             // e.g. "down for 2h 14m · not acknowledged" / "up · updated 3m ago"
    schedule: String,          // schedule_label(&check)
    ping_url: String,
    bars: Vec<crate::view::Bar>,
    channel_boxes: Vec<ChannelBox>,
    pings: Vec<PingRow>,
    notifications: Vec<NotificationRow>,
}
```

In `check_show`, after loading pings (raise the limit to `40` so pairing has context; detail heartbeat window `n = 30`):

```rust
let now = Utc::now();
let recent = state.store.list_recent_pings(id, 40).await?;
let durations = crate::view::run_durations(&recent);
let bars = crate::view::heartbeat(&recent, check.max_runtime_secs, check.status == CheckStatus::Paused, 30);
let pings = recent.iter().take(20).map(|p| PingRow {
    time: p.created_at.format("%H:%M:%S").to_string(),
    kind: p.kind.as_str(),
    exit: p.exit_code.map(|c| format!("exit {c}")).unwrap_or_else(|| "—".into()),
    duration: durations.get(&p.id).map(|d| crate::view::fmt_secs(*d)).unwrap_or_else(|| "—".into()),
    source: p.source_ip.clone().unwrap_or_else(|| "—".into()),
    body: p.body.clone(),
}).collect();
let status = crate::view::display_status(&check, now).as_str();
let since = /* build from status + last_ping_at + acknowledged, e.g. */ status_since_label(&check, now);
```

Add a small `status_since_label(check, now) -> String` helper (down → "down · " + fmt_relative(last_ping_at); acknowledged suffix "· not acknowledged"/"· acknowledged"; else "updated " + relative).

- [ ] **Step 4: Rewrite `templates/check.html`** from `check-detail.html`. Sections in order: breadcrumb (`Checks / <project> / <name>` — link project to `/projects/{{ check.project_id }}`), `.chead` (status-dot + `{{ check.name }}` + `.badge {{ status }}` + `.since`), `.actions` (keep every existing action + method/POST action URL: `Edit`→`/checks/{{ check.id }}/edit`; Pause/Resume conditional on `check.status`; `Acknowledge` shown when `status == "down"` and `!check.acknowledged`; `Regenerate URL`; `Delete` with `onsubmit="return confirm('Delete this check?')"`). Ping-URL card with `<code>{{ ping_url }}</code>` + a `.copy` button. `.beat` strip: `{% for b in bars %}<i class="{{ b.class }}" style="height:{{ b.height }}px" title="{{ b.title }}"></i>{% endfor %}`. Channels `.card`: keep the existing `POST /checks/{{ check.id }}/channels` form with `.chk` checkbox labels (`name="channel_ids"` values, `checked` when `cb.bound`). Recent pings `.card` table with columns **When · Kind · Exit · Duration · Source**; render each row, and when `p.body` is non-empty emit the toggle caret + a following `tr.exp` output row:

```html
{% for p in pings %}
{% if p.body.is_empty() %}
<tr><td class="mono"><span class="caret" style="opacity:0">▸</span>{{ p.time }}</td><td><span class="pill {{ p.kind }}">{{ p.kind }}</span></td><td class="mono">{{ p.exit }}</td><td class="mono">{{ p.duration }}</td><td class="mono">{{ p.source }}</td></tr>
{% else %}
<tr class="toggle" style="cursor:pointer"><td class="mono"><span class="caret">▸</span>{{ p.time }}</td><td><span class="pill {{ p.kind }}">{{ p.kind }}</span></td><td class="mono">{{ p.exit }}</td><td class="mono">{{ p.duration }}</td><td class="mono">{{ p.source }}</td></tr>
<tr class="exp"><td colspan="5" style="border-top:none;padding-top:0"><div class="out {{ p.kind }}"><span class="tag">captured output · POST body</span>{{ p.body }}</div></td></tr>
{% endif %}
{% endfor %}
```

Recent notifications `.card` table (When · Event · Channel · Result) as today, restyled. Append the page-local JS for caret toggle + copy button (from `check-detail.html`'s script + a `navigator.clipboard.writeText` on `.copy`).

> Askama note: `{% if p.body.is_empty() %}` calls a method on a `String` field — valid in askama 0.16. Body is auto HTML-escaped (safe for arbitrary POST content).

- [ ] **Step 5: Run tests** — `cargo fmt && cargo nextest run` → check_detail PASS; suite green.

- [ ] **Step 6: Commit** — `git add src/web.rs templates/check.html tests/check_detail.rs && git commit -m "feat(ui): redesign check detail; surface ping body/source/duration"`

---

### Task 6: Forms — check, project, channel

Restyle against `.field`/`.btn` classes; preserve every input `name`, option `value`, the channel kind-toggle JS, and `smtp_available` gating.

**Files:**
- Modify: `templates/check_form.html`, `templates/project_form.html`, `templates/channel_form.html`
- Test: extend `tests/check_detail.rs` (or a new `tests/forms_view.rs`)

- [ ] **Step 1: Failing test** — assert the channel form still exposes its fields and the email option gate, now restyled. Reuse the existing pattern from `tests/auth_web.rs` (which already checks `value="email"` presence/absence by `smtp_available`). Add one assertion that a form control class is present:

```rust
let res = server.get(&format!("/projects/{pid}/channels/new")).await;
let body = res.text();
assert!(body.contains("class=\"field\""), "form not restyled");
assert!(body.contains("name=\"webhook_url\""), "webhook field lost");
```

- [ ] **Step 2: Run it, verify it fails** — `cargo nextest run --test forms_view` → FAIL.

- [ ] **Step 3: Restyle `templates/check_form.html`** — wrap each existing control in `<div class="field"><label>…</label><input …><div class="help">…</div></div>`. Keep ALL fields and `name=` exactly: `name`, `schedule_kind` (select period/cron), `period_secs`, `cron_expr`, `grace_secs`, `timezone`, `scan_interval_secs`, `max_runtime_secs`, `nag_interval_secs`. Keep `{% if let Some(error) = error %}` → render as a styled error banner (`<p class="flash err">`). Submit `<button class="btn primary">Save changes</button>`.

- [ ] **Step 4: Restyle `templates/project_form.html`** — same treatment for `name`, `scan_interval_secs`, `nag_interval_secs`.

- [ ] **Step 5: Restyle `templates/channel_form.html`** — wrap in `.field`s; keep the `<select id="kind">` with all `value=`s and the `{% if smtp_available %}` email option; keep each `.cfg[data-kind]` block and the existing `<script>` that shows only the active kind's fields (unchanged logic). Restyle the error banner.

- [ ] **Step 6: Run tests** — `cargo fmt && cargo nextest run` → PASS (incl. the pre-existing email-gating tests in `tests/auth_web.rs`).

- [ ] **Step 7: Commit** — `git add templates/check_form.html templates/project_form.html templates/channel_form.html tests/forms_view.rs && git commit -m "feat(ui): restyle check/project/channel forms"`

---

### Task 7: Project page, settings, users

**Files:**
- Modify: `templates/project.html`, `templates/settings.html`, `templates/users.html`
- Test: extend an existing web test (assert restyle markers; keep behaviour strings)

- [ ] **Step 1: Failing test** — assert `/settings` and `/users` render with `.card`/`.field` and that the project page keeps `Send test` and `Test notification sent`/`failed` banner classing. Reuse the channel-test flow already covered in `tests/auth_web.rs`; add:

```rust
let res = server.get("/settings").await; // as admin
assert!(res.text().contains("class=\"field\""));
```

- [ ] **Step 2: Run it, verify it fails.**

- [ ] **Step 3: Restyle `templates/project.html`** — page header (`{{ project.name }}`) + action links (`Edit`, `New check`, `New channel`, `Delete project` with confirm). Style the `test_result` banner as `<p class="flash {% if tr.ok %}ok{% else %}err{% endif %}">{{ tr.message }}</p>` (keep the exact message text). Checks `.list` (reuse `.check` row style; link to `/checks/{{ c.id }}`, status via `c.status.as_str()` — stored status here is fine, or compute display status if you thread `now` in; stored is acceptable on this page). Channels `.card` with per-row `Send test` (`POST /channels/{{ ch.id }}/test`) and `delete` (`POST /channels/{{ ch.id }}/delete`) buttons.

- [ ] **Step 4: Restyle `templates/settings.html`** — `.field`s for `scan_interval`, `nag_interval`, `pings_retention_days`, `notifications_retention_days`; `Save changes` primary button; keep the `Manage users` link to `/users`.

- [ ] **Step 5: Restyle `templates/users.html`** — restyle the users `table` and the add-user `.field` form (`username`, `password`, `is_admin` checkbox with value `1`). Keep the error banner.

- [ ] **Step 6: Run tests** — `cargo fmt && cargo nextest run` → green.

- [ ] **Step 7: Commit** — `git add templates/project.html templates/settings.html templates/users.html && git commit -m "feat(ui): restyle project, settings, and users pages"`

---

### Task 8: Login & setup (unauthenticated, centered card)

These render with `show_nav = false` — no top bar. Center a narrow branded card.

**Files:**
- Modify: `templates/login.html`, `templates/setup.html`, `assets/app.css` (add `.auth` centering + `.flash` if not already present)
- Test: extend `tests/auth_web.rs` if it asserts on these pages; otherwise a minimal GET check.

- [ ] **Step 1: Failing test** — `GET /login` (with a user present) contains `class="auth"`; error path still contains `invalid username or password` after a bad POST (existing behaviour).

- [ ] **Step 2: Run it, verify it fails.**

- [ ] **Step 3: Add `.auth` card styles** to `assets/app.css` (centered fl‑column, `max-width:360px`, brandmark, `.flash.err`/`.flash.ok` message styles) — only if not already added in Task 5.

- [ ] **Step 4: Restyle `templates/login.html`** — `.auth` card, `▚ pingward` brandmark, `.field`s for `username`/`password`, `Log in` primary button, styled error. Keep the string `invalid username or password` reachable via the handler (already in `web.rs`).

- [ ] **Step 5: Restyle `templates/setup.html`** — same card; heading `Create the first admin`; `Create admin` button; keep field names `username`/`password`.

- [ ] **Step 6: Run tests** — `cargo fmt && cargo nextest run` → green.

- [ ] **Step 7: Commit** — `git add templates/login.html templates/setup.html assets/app.css tests/auth_web.rs && git commit -m "feat(ui): restyle login and setup"`

---

### Task 9: Verification pass — a11y, responsive, real-app smoke

**Files:** none (or tiny `assets/app.css` fixes surfaced by review)

- [ ] **Step 1: Reduced motion + focus** — confirm `assets/app.css` has the `prefers-reduced-motion` block and a visible focus ring on links/buttons/inputs (`:focus-visible` outline using `--brand`). Add if missing.

- [ ] **Step 2: Responsive** — resize check at ~360px: page never scrolls horizontally; the check-row `.spark` and `.cwhen` collapse (mockup `@media(max-width:640px)`), tables/`.out` scroll within their container. Fix in CSS if needed.

- [ ] **Step 3: Contrast** — spot-check body text and status colours on their surfaces in both themes (AA). Adjust token if any pair fails.

- [ ] **Step 4: Real-app smoke** — invoke the `/run` skill (or `cargo run`), create a project + a check, send `curl` pings (success with body, start→success pair, fail with body, `/log` with body), and confirm in a browser: heartbeat renders (hollow / measured / hot / bad / paused), late state appears in the grace window, ping bodies expand collapsed-by-default, copy button works, theme toggle persists across reload. Capture a screenshot of dashboard + detail in both themes.

- [ ] **Step 5: Full suite + fmt** — `pwd && cargo fmt --check && cargo nextest run` → all green.

- [ ] **Step 6: Commit any fixes** — `git add <changed> && git commit -m "fix(ui): a11y and responsive polish"`

---

## Self-Review

**Spec coverage:** §3 tech/personality → Tasks 1,4,5. §4 tokens → Task 1. §5 fonts → Tasks 1–2. §6 status model (`late`/`new`, tiles Total/Up/Late/Down) → Tasks 3,4. §7 heartbeat (formula, ceiling+window fallback, `<2` guard, hot, hollow, paused flatline, duration pairing) → Task 3, applied 4/5. §8 body/source/log + collapsed-by-default → Task 5. §9 components → Tasks 1,4,5,6,7,8. §10 per-page → Tasks 4–8. §11 a11y/responsive → Task 9. §12 testing → each task's tests + Task 3 units. §13 scope (follow-ups excluded) → honored. §14 resolved decisions (self-host fonts, N=6/30, new→Total) → Tasks 2,4.

**Placeholder scan:** No TBD/TODO. CSS "port from mockups" points at concrete on-disk artifacts with an explicit class inventory (Task 1 Step 5), not a vague "style it nicely". All Rust logic is inlined in full.

**Type consistency:** `Bar { height:u32, class:&'static str, title:String }` and `heartbeat(pings, max_runtime_secs, paused, n)` used identically in Tasks 3/4/5. `DisplayStatus::as_str()` feeds both the `status-dot`/`badge` class and text. `run_durations` returns `HashMap<i64,i64>` keyed by finish ping id, consumed the same way in Task 5. `schedule_label`/`status_since_label` defined in Task 4/5 web.rs.
