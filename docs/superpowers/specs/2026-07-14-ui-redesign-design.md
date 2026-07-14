# Pingward UI Redesign — Design Spec

- **Date:** 2026-07-14
- **Branch:** `feat/ui-redesign`
- **Status:** Approved for planning (visual direction validated in the brainstorm companion)

## 1. Context

Pingward is a self-hosted cron / heartbeat monitor (Healthchecks.io-style). Jobs
ping a URL; a check goes `down` when a ping is overdue past its grace window.
Hierarchy is Project → Check, with notification channels (webhook, Slack,
Telegram, ntfy, Pushover, email), plus users and settings.

The current UI is a few lines of inline CSS in `templates/base.html` over raw
tables and forms — no design language. This redesign gives it a distinctive,
coherent visual identity while staying inside the existing server-rendered
stack.

## 2. Goals / Non-goals

**Goals**
- One coherent design system applied across every page.
- Light **and** dark themes.
- Surface data the backend already captures but the UI hides today: per-ping
  request **body**, **source IP**, and the **`log`** ping kind.
- A signature "heartbeat" visualization that reads at a glance and stays honest
  about what data exists.

**Non-goals (explicit YAGNI)**
- No SPA / JS framework, no JSON API. Stays server-rendered.
- No new notification channels, scheduling features, or auth changes.
- No schema migrations *for styling*. The only data-shape work is deriving
  per-ping durations (§7) and the derived `late` display state (§6) — both
  computed at read time, no new columns.
- No dependency on external CDNs at runtime.

## 3. Locked design decisions

| Axis | Decision |
| --- | --- |
| Tech | Server-rendered `askama` templates + a real CSS design system + minimal vanilla JS. |
| Personality | **Console** — ops/NOC feel: monospace-forward data, glowing status lights, calm-when-healthy. |
| Theming | Dual theme. **Light** uses the indigo "Pulse" palette; **dark** uses the cyan "Console" palette. Toggle in the top bar; default follows `prefers-color-scheme`; choice persisted in `localStorage`. |
| Brand accent | Per-theme: indigo `#5654D4` (light), cyan `#2FE3CE` (dark). Deliberately distinct from status colors so it never collides with up/down/late. |
| Signature | The **heartbeat bar** — a per-check strip of recent runs where bar height = fraction of runtime budget used (§7). Healthy runs sit low and quiet; runs creeping toward timeout grow and turn amber. |

## 4. Design tokens

Delivered as CSS custom properties on `:root` (dark) and `:root[data-theme="light"]`.

**Dark (Console)**
```
--brand:#2FE3CE  --brand-ink:#062b27
--bg:#0B0E15  --surface:#0F1420  --surface-2:#131A28  --hover:#151d2e  --input:#0a0d14
--border:#1E2534  --border-soft:#161d2b
--ink:#E9EDF4  --ink-2:#C4CCDA  --muted:#7C8AA0  --faint:#5A6678
--up:#37D6A3  --down:#FF5C6C  --late:#F5B13C  --paused:#5A6678  --new:#7C8AA0
--bar-none:#33405a  --glow:.62
```

**Light (Pulse)**
```
--brand:#5654D4  --brand-ink:#ffffff
--bg:#F3F5F8  --surface:#FFFFFF  --surface-2:#FFFFFF  --hover:#F7F8FA  --input:#FBFCFD
--border:#E4E7EC  --border-soft:#EEF0F3
--ink:#171A21  --ink-2:#2C3240  --muted:#727A8A  --faint:#9AA1B0
--up:#0E7C5A  --down:#D23B48  --late:#B26F12  --paused:#8A94A6  --new:#727A8A
--bar-none:#CBD3DE  --glow:0
```

Each status also has soft `-bg` / `-bd` variants for badges (see mockups).

**Type**
- UI / body: `Inter`, fallback `system-ui, sans-serif`.
- Data / mono (URLs, times, schedules, exit codes, numbers, output): `IBM Plex Mono`, fallback `ui-monospace, SFMono-Regular, Menlo, Consolas, monospace`.
- Summary-tile numbers and monospace data are tabular for column alignment.

**Radius:** cards `13px`, controls/inputs `9–10px`, badges/pills `7px`, dots round.
**Shadow:** one elevation token per theme, used on list cards.
**Backdrop:** subtle dotted grid on the page background (very low-contrast).

## 5. Font & asset delivery (decision for review)

The current app inlines CSS in `base.html` and serves no static assets.

**Proposed:** add a small static-asset route (embedded, no build step) serving:
- `app.css` — the full design system, linked from `base.html` (replaces the inline `<style>`).
- Two self-hosted `woff2` families, **Inter** and **IBM Plex Mono** (latin subset), so the approved look renders without any CDN, with the system stacks above as fallback.

**Alternative (lighter):** ship **no web fonts** and rely on `system-ui` +
`ui-monospace`. Zero binary assets; the console feel largely survives on the
system monospace, but loses the exact Inter/Plex texture the mockups were
approved with.

> **Resolved:** self-host the two woff2 families (Inter + IBM Plex Mono, latin
> subset), with system-stack fallbacks. No runtime CDN.

## 6. Status model

Stored `CheckStatus` is `new | up | down | paused` (no `late`). The UI presents
five display states; `late` is **derived at render time**, never stored:

| Display state | Meaning | Derivation |
| --- | --- | --- |
| `new` | Created, never pinged | `status == New` |
| `up` | Healthy | `status == Up` and not late |
| `late` | Overdue but still in grace window | `status == Up` and `now > next_due_at - grace_secs` and `now ≤ next_due_at` |
| `down` | Overdue past grace / last ping failed | `status == Down` |
| `paused` | Monitoring off | `status == Paused` |

`next_due_at` already bakes in grace (`scheduler.rs`), so `next_due_at - grace_secs`
is the expected run time; between it and `next_due_at` the check is "running late".
Derivation lives in a small view-model helper, not in the store.

Dashboard summary tiles: **Total · Up · Late · Down**. `new` counts in Total and
shows a `new` badge in the list; `paused` is excluded from Up/Down counts.

## 7. Heartbeat bar — the signature

A horizontal strip of the last *N* runs for a check (dashboard `N≈6`, detail `N≈30`).

**Per-bar height**
```
height = clamp(minH, maxH, maxH * (duration / ceiling))
  ceiling = max_runtime_secs         if set
          = max(durations in window) otherwise (relative fallback)
```
- **Shorter = ran fast / lots of headroom.** **Taller = ran longer / near timeout.**
  Intuitive reading: a near-full bar means "it ran almost until the timeout".
- Healthy jobs sit low and quiet; only runs approaching the limit grow tall and
  draw the eye — matching the product's "quiet when healthy, loud when not" job.

**Per-bar colour** encodes the *outcome*, independent of height:
- success → `--up`
- success using **≥ 80 %** of `max_runtime_secs` → `--late` (amber, "ran hot")
- fail → `--down`

**Duration derivation.** Durations are not stored. A run's duration = the finish
ping's `created_at` minus the preceding `start` ping's `created_at`, paired in
time order per check. Needs a store method returning recent pings for a check;
pairing happens in a view-model pass. This is the one piece of non-trivial
read-side backend work in this redesign.

**Bars without a measurable duration**
- **No duration** (job never sent a `start`, or no matching pair): **hollow bar**
  — faint fill + visible outline, fixed neutral height. Says "pinged, duration
  unknown"; never implies a magnitude.
- **Relative-fallback guard:** if the ceiling would come from the window and there
  are **fewer than 2** durations, render hollow instead (a lone value would
  otherwise map to full height and look falsely alarming).
- **Paused:** a low, dimmed **flatline** of equal short bars — clearly "not
  monitoring", distinct in both fill (solid vs hollow) and height (short vs tall)
  from the no-duration bar.

A compact legend accompanies the dashboard strip.

## 8. Surfacing hidden data (ping body / source IP / log)

The backend already stores `pings.body` (POST body, ≤ 10 KiB), `pings.source_ip`,
and supports the `log` ping kind (records body, does not change status). None is
shown today. The redesigned **Recent pings** table (check detail) exposes them:

- Columns: **When · Kind · Exit · Duration · Source**.
- `Kind` pills for `success | fail | start | log`.
- **Any** ping whose `body` is non-empty gets a ▸ toggle; **collapsed by default**
  (keeps the list short), click the row to expand a monospace output panel that
  preserves whitespace. Kind is signalled by a coloured left border
  (green/red/brand). Rows with empty body have no toggle.

Pure template + a few lines of vanilla JS; the data is already persisted.

## 9. Component inventory

Shared via `base.html` + `app.css`, reused across pages:

- **Top bar** — brandmark `▚ pingward` (mono), nav links, theme toggle, log out.
- **Page header** — title, subtitle, primary action button.
- **Summary tiles** — number + label + status edge stripe.
- **Project group** — heading with count + rule + "Manage".
- **List card** + **check row** — status dot (glowing; `up`/`down` ripple, respects
  reduced-motion), name, mono schedule, heartbeat strip, relative time, status badge.
- **Status badge** — fixed `min-width` so `up/late/down/paused/new` align.
- **Buttons** — `primary` (brand), default, `danger` (red), `warn` (amber).
- **Form controls** — labelled inputs/selects with help text and brand focus ring;
  custom checkbox; the channel form keeps its kind-driven field toggling.
- **Tables** — uppercase faint headers, hairline row separators, mono data cells.
- **Expandable output panel** (§8).
- **Breadcrumb** — Checks / Project / Check on detail pages.
- **Flash / error** — inline message styled by status colour (login/setup/forms).

## 10. Per-page application

| Template | Notes |
| --- | --- |
| `base.html` | New shell: link `app.css`, top bar, theme bootstrap JS, token `<style>` (or served CSS). `show_nav` still gates authed nav. |
| `dashboard.html` | Summary tiles + project groups of list cards + heartbeat legend. |
| `check.html` | Breadcrumb, status header, action buttons, ping-URL row w/ Copy, large heartbeat, channel checkboxes, Recent pings (with body expand), Recent notifications. |
| `check_form.html` | Restyled fields; keep all existing inputs (schedule kind, period/cron, grace, timezone, intervals, max runtime, nag). Error banner styled. |
| `project.html` | Project header + actions; Checks list card; Channels list card with per-row Send test / delete; `test_result` flash styled. |
| `project_form.html` | Restyled name + interval fields. |
| `channel_form.html` | Restyled; keep kind `<select>` + JS that shows only the active kind's fields; keep `smtp_available` gating of the email option. |
| `settings.html` | Restyled global scan/nag/retention fields; link to Manage users. |
| `users.html` | Restyled users table + add-user form. |
| `login.html` / `setup.html` | Centered narrow card, brand mark, styled error. |

No page introduces a component not listed in §9.

## 11. Accessibility & responsiveness

- Colour is never the sole signal: status has dot **+** text badge; heartbeat has
  height **+** colour **+** tooltip.
- Visible keyboard focus on all interactive elements (brand focus ring).
- `prefers-reduced-motion`: disable dot ripple and transitions.
- Contrast: body text and status colours meet WCAG AA on their surfaces in both
  themes (verify during build).
- Responsive: single-column below ~720px; heartbeat strip and secondary columns
  drop on narrow check rows; page body never scrolls horizontally (wide tables /
  output panels scroll within their own container).

## 12. Testing

- Existing route/integration tests (`axum-test`) assert behaviour and should keep
  passing; update any that assert on specific old markup.
- Add coverage for the new read-side logic that has real branching:
  - derived `late` state (in-grace vs past-grace vs new).
  - heartbeat duration pairing + ceiling selection + the `<2 durations` hollow guard.
- Rust tests via `cargo nextest run`; `cargo fmt` before commit.

## 13. Scope & optional follow-ups

**In scope (first version):** design system + dual theme + all templates restyled +
heartbeat bars + status model (`late`/`new`) + hidden-data surfacing (body/source/log,
collapsed).

**Optional follow-ups (not now):**
- A dedicated runtime sparkline (absolute seconds) on the detail page for checks
  that send `start` pings.
- Filter/search on the dashboard.
- Live refresh (poll / SSE) of statuses.

## 14. Resolved decisions (from spec review)

1. **Fonts:** self-host Inter + IBM Plex Mono woff2 (latin subset), system-stack fallback.
2. **Heartbeat window:** dashboard `N=6`, detail `N=30`.
3. **`new` in summary tiles:** folds into **Total** (no dedicated tile); new checks
   show a `new` badge in the list. Summary tiles stay **Total · Up · Late · Down**.
