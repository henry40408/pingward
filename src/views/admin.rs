use maud::{Markup, html};

use super::layout::{Chrome, csrf_field, page};
use crate::models::{Check, Notification, Project};

/// Secrets are redacted in Rust before reaching here, so no view can print one.
pub struct EnvSetting {
    pub var: &'static str,
    pub value: EnvValue,
    pub default: &'static str,
    pub description: &'static str,
}

/// `Secret` carries only whether something is configured, never the value.
pub enum EnvValue {
    Set(String),
    Unset,
    Secret(bool),
}

/// Plus `is_self`, so the admin's own row renders its self-mutation controls inert.
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub is_admin: bool,
    pub disabled: bool,
    pub is_self: bool,
}

/// The whole `/admin` page. Colliding names take a section prefix (`settings_*`,
/// `user_*`, `user_count`/`project_count`).
pub struct AdminView {
    pub csrf: String,
    /// The history fragment, composed as `Markup` so `/admin` and `/admin/audit`
    /// emit identical markup.
    pub audit_partial: Markup,
    // overview
    pub user_count: i64,
    pub project_count: i64,
    pub checks: i64,
    pub pings_24h: i64,
    pub status: crate::store::CheckStatusCounts,
    pub down: Vec<(Check, String, String)>,
    pub notif_ok: i64,
    pub notif_err: i64,
    pub channel_fail: Vec<(String, i64, i64)>,
    pub recent_fail: Vec<Notification>,
    pub last_scan_at: Option<String>,
    pub last_prune_at: Option<String>,
    /// "3m ago", rendered server-side for no-JS; `app.js` re-ticks via `data-ago`.
    pub last_scan_ago: Option<String>,
    pub last_prune_ago: Option<String>,
    /// The same stamps as readable UTC; `data-ts`/`data-ago` keep the RFC3339.
    pub last_scan_utc: Option<String>,
    pub last_prune_utc: Option<String>,
    // settings
    pub scan_interval: String,
    pub nag_interval: String,
    pub pings_retention_days: String,
    pub notifications_retention_days: String,
    pub audit_retention_days: String,
    pub display_timezone: String,
    pub settings_error: Option<String>,
    pub settings_flash: Option<String>,
    // users
    pub users: Vec<UserRow>,
    pub user_flash: Option<String>,
    pub elevation_flash: Option<String>,
    /// False for a passwordless forward-auth admin (nothing to re-assert).
    pub elevation_applies: bool,
    /// `Some(readable duration)` while the unlock is live, `None` when locked.
    pub elevation_remaining: Option<String>,
    /// Applies and unconfirmed; only then do gated controls carry `data-reauth`. The
    /// server re-checks, so a lapse before the click just bounces to `/admin/unlock`.
    pub elevation_locked: bool,
    pub password_reset_flash: Option<String>,
    pub user_error: Option<String>,
    pub projects: Vec<(Project, String)>,
    pub env_rows: Vec<(&'static str, Vec<EnvSetting>)>,
}

pub fn admin(view: &AdminView) -> Markup {
    let body = html! {
        div class="phead" { div {
            h1 { "Admin" }
            p class="crumb tight" { "Site-wide overview, global settings, user management, and every project across all users." }
        } }
        (scale_tiles(view))
        (check_health_card(view))
        (notification_health_card(view))
        (scheduler_card(view))
        (settings_card(view))
        (environment_card(view))
        @if view.elevation_applies { (elevation_card(view)) }
        (users_card(view))
        (projects_group(view))
        div class="card" {
            div class="ch" { h2 { "Audit trail" } }
            div class="cb" id="audit-section" data-endpoint="/admin/audit" { (view.audit_partial) }
        }
    };
    page(
        &Chrome {
            show_nav: true,
            is_admin: true,
            csrf: &view.csrf,
        },
        body,
    )
}

fn scale_tiles(view: &AdminView) -> Markup {
    // Bare .tiles, as on the dashboard: a tile is already a card.
    let tile = |n: i64, label: &str| {
        html! { div class="tile total" { span class="edge" {} div class="n" { (n) } div class="l" { (label) } } }
    };
    html! {
        div class="tiles" data-testid="admin-scale" {
            (tile(view.user_count, "Users"))
            (tile(view.project_count, "Projects"))
            (tile(view.checks, "Checks"))
            (tile(view.pings_24h, "Pings (24h)"))
        }
    }
}

fn check_health_card(view: &AdminView) -> Markup {
    let s = &view.status;
    html! {
        div class="card" {
            div class="ch" { h2 { "Check health" } }
            div class="cb" {
                p class="pillrow" {
                    span class="badge new" { (s.new) " new" }
                    span class="badge up" { (s.up) " up" }
                    span class="badge running" { (s.running) " running" }
                    span class="badge down" { (s.down) " down" }
                    span class="badge paused" { (s.paused) " paused" }
                }
                @if view.down.is_empty() {
                    p class="note" { "No checks are currently down." }
                } @else {
                    div class="tscroll" { table data-testid="health-down" {
                        thead { tr { th { "Check" } th { "Project" } th { "Owner" } } }
                        tbody {
                            @for (check, project, owner) in &view.down {
                                tr {
                                    td { a href={"/admin/checks/" (check.id)} { (check.name) } }
                                    td { (project) }
                                    td { (owner) }
                                }
                            }
                        }
                    } }
                }
            }
        }
    }
}

fn notification_health_card(view: &AdminView) -> Markup {
    html! {
        div class="card" {
            div class="ch" { h2 { "Notification health (24h)" } }
            div class="cb" {
                p class="pillrow" {
                    span class="pill ok" { (view.notif_ok) " ok" }
                    span class="pill fail" { (view.notif_err) " error" }
                }
                @if view.channel_fail.is_empty() {
                    p class="note" { "No notifications sent in the last 24h." }
                } @else {
                    div class="tscroll" { table data-testid="health-channels" {
                        thead { tr { th { "Channel" } th { "Ok" } th { "Error" } } }
                        tbody {
                            @for (name, ok, err) in &view.channel_fail {
                                tr { td { (name) } td class="mono ok-txt" { (ok) } td class="mono fail-txt" { (err) } }
                            }
                        }
                    } }
                }
                h2 class="subhead" { "Recent failures" }
                @if view.recent_fail.is_empty() {
                    p class="note" { "No recent notification failures." }
                } @else {
                    div class="tscroll" { table data-testid="health-recent" {
                        thead { tr { th { "When" } th { "Event" } th { "Check" } th { "Error" } } }
                        tbody {
                            @for n in &view.recent_fail {
                                tr {
                                    td class="mono" { span class="localtime" data-ts=(n.created_at.to_rfc3339()) { (crate::view::fmt_utc(&n.created_at)) } }
                                    td { (n.event.as_str()) }
                                    td { a href={"/admin/checks/" (n.check_id)} { "#" (n.check_id) } }
                                    td class="fail-txt" { @if let Some(e) = &n.error { (e) } }
                                }
                            }
                        }
                    } }
                }
            }
        }
    }
}

struct LoopStamp<'a> {
    testid: &'static str,
    label: &'static str,
    at: Option<&'a str>,
    utc: Option<&'a str>,
    ago: Option<&'a str>,
}

fn loop_stamp(stamp: &LoopStamp<'_>) -> Markup {
    html! {
        div class="hb" data-testid=(stamp.testid) {
            span class={"hb-dot" @if stamp.at.is_none() { " idle" }} {}
            div class="hb-body" {
                div class="hb-label" { (stamp.label) }
                @if let Some(t) = stamp.at {
                    div class="hb-time localtime" data-ts=(t) { (stamp.utc.unwrap_or(t)) }
                    div class="hb-ago" data-ago=(t) { @if let Some(a) = stamp.ago { (a) } }
                } @else {
                    div class="hb-time hb-never" { "never" }
                }
            }
        }
    }
}

fn scheduler_card(view: &AdminView) -> Markup {
    html! {
        div class="card" {
            div class="ch" { h2 { "Scheduler" } }
            div class="cb" { div class="heartbeats" {
                (loop_stamp(&LoopStamp {
                    testid: "sched-scan",
                    label: "Last scan",
                    at: view.last_scan_at.as_deref(),
                    utc: view.last_scan_utc.as_deref(),
                    ago: view.last_scan_ago.as_deref(),
                }))
                (loop_stamp(&LoopStamp {
                    testid: "sched-prune",
                    label: "Last prune",
                    at: view.last_prune_at.as_deref(),
                    utc: view.last_prune_utc.as_deref(),
                    ago: view.last_prune_ago.as_deref(),
                }))
            } }
        }
    }
}

fn settings_card(view: &AdminView) -> Markup {
    html! {
        div class="card" {
            div class="ch" { h2 { "Settings" } }
            div class="cb" {
                @if let Some(error) = &view.settings_error { p class="flash err" { (error) } }
                @if let Some(f) = &view.settings_flash { p class="flash ok" data-testid="settings-flash" { (f) } }
                form method="post" action="/admin/settings" {
                    (csrf_field(&view.csrf))
                    // Not for the retention fields below: those are plain day counts.
                    datalist id="dur-list" {
                        @for d in crate::view::durations() { option value=(d) {} }
                    }
                    div class="two" {
                        div class="field" {
                            label for="scan_interval" { "Global scan interval" }
                            input id="scan_interval" name="scan_interval" value=(view.scan_interval) list="dur-list" autocomplete="off";
                            div class="help" { "Seconds, or a duration like 5m, 1h30m." }
                        }
                        div class="field" {
                            label for="nag_interval" { "Global nag interval" }
                            input id="nag_interval" name="nag_interval" value=(view.nag_interval) list="dur-list" autocomplete="off";
                            div class="help" { "Leave blank to turn off nagging. Seconds, or a duration like 5m, 1h30m." }
                        }
                    }
                    div class="two" {
                        div class="field" {
                            label for="pings_retention_days" { "Pings retention (days)" }
                            input id="pings_retention_days" name="pings_retention_days" value=(view.pings_retention_days);
                            div class="help" { "Leave blank to keep forever." }
                        }
                        div class="field" {
                            label for="notifications_retention_days" { "Notifications retention (days)" }
                            input id="notifications_retention_days" name="notifications_retention_days" value=(view.notifications_retention_days);
                            div class="help" { "Leave blank to keep forever." }
                        }
                    }
                    div class="two" {
                        div class="field" {
                            label for="audit_retention_days" { "Audit trail retention (days)" }
                            input id="audit_retention_days" name="audit_retention_days" value=(view.audit_retention_days) data-testid="audit-retention-input";
                            div class="help" { "Blank keeps every entry forever — nothing else trims this table, so it grows for the life of the instance. Shortening it deletes history from the audit trail below, and the change is itself recorded there." }
                        }
                        div class="field" {
                            label for="display_timezone" { "Notification timezone" }
                            input id="display_timezone" name="display_timezone" value=(view.display_timezone) list="tz-list" autocomplete="off" data-testid="display-timezone-input";
                            datalist id="tz-list" {
                                @for tz in crate::view::timezones() { option value=(tz) {} }
                            }
                            div class="help" { "Timestamps in delivered notifications render in this zone. Blank uses each check's own timezone (itself UTC unless set). Pages in this UI always follow your browser instead." }
                        }
                    }
                    div class="btnrow" { button class="btn primary" type="submit" { "Save changes" } }
                }
            }
        }
    }
}

fn environment_card(view: &AdminView) -> Markup {
    html! {
        div class="card" {
            div class="ch" { h2 { "Environment" } }
            div class="cb" {
                p class="crumb tight" { "Configured via environment variables — changing these requires restarting the server." }
                @for (group, rows) in &view.env_rows {
                    h2 class="subhead" { (group) }
                    div class="tscroll" { table class="envtable" {
                        thead { tr { th { "Variable" } th { "Current" } th { "Default" } th { "Description" } } }
                        tbody {
                            @for row in rows { (env_row(row)) }
                        }
                    } }
                }
            }
        }
    }
}

fn env_row(row: &EnvSetting) -> Markup {
    let smtp_password = (row.var == "PINGWARD_SMTP_PASSWORD").then_some("env-smtp-password");
    html! {
        tr data-testid={"env-row-" (row.var)} {
            td class="mono" { (row.var) }
            td data-testid=[smtp_password] {
                @match &row.value {
                    EnvValue::Set(v) => { code { (v) } }
                    EnvValue::Unset => { span class="note" { "not set" } }
                    EnvValue::Secret(true) => { span class="pill ok" { "configured" } }
                    EnvValue::Secret(false) => { span class="note" { "not set" } }
                }
            }
            td class="mono" { (row.default) }
            td { (row.description) }
        }
    }
}

fn elevation_card(view: &AdminView) -> Markup {
    html! {
        div class="card" {
            div class="ch" { h2 { "Admin actions" } }
            div class="cb" {
                @if let Some(f) = &view.elevation_flash { p class="flash ok" data-testid="elevation-flash" { (f) } }
                @if let Some(left) = &view.elevation_remaining {
                    p class="note" data-testid="elevation-state" { "Confirmed — creating a user, resetting a password and granting admin stay available for another " (left) "." }
                } @else {
                    p class="crumb tight" data-testid="elevation-state" { "Creating a user, resetting a password and granting admin each hand out access that outlasts this browser session, so they ask you to confirm your password first. Removing access — disabling, demoting, deleting — never does." }
                    p { a class="btn" href="/admin/unlock" data-testid="elevation-confirm-link" { "Confirm now" } }
                }
            }
        }
    }
}

fn users_card(view: &AdminView) -> Markup {
    html! {
        div class="card" {
            div class="ch" { h2 { "All users" } }
            div class="cb" {
                @if let Some(f) = &view.user_flash { p class="flash err" data-testid="users-flash" { (f) } }
                @if let Some(f) = &view.password_reset_flash { p class="flash warn" data-testid="password-reset-flash" { (f) } }
                // .tscroll keeps the Add-user form below from scrolling with the table.
                div class="tscroll" { table {
                    thead { tr { th { "Username" } th { "Admin" } th {} } }
                    tbody {
                        @for u in &view.users { (user_row(u, view)) }
                    }
                } }
                h2 class="subhead" { "Add user" }
                @if let Some(error) = &view.user_error { p class="flash err" data-testid="user-error" { (error) } }
                (add_user_form(view))
            }
        }
    }
}

fn user_row(u: &UserRow, view: &AdminView) -> Markup {
    let csrf = &view.csrf;
    let locked = view.elevation_locked;
    let toggle_admin_label = if u.is_admin {
        "revoke admin"
    } else {
        "make admin"
    };
    let toggle_disabled_label = if u.disabled { "enable" } else { "disable" };
    // Granting admin is gated on elevation; revoking is not, so it confirms instead.
    let admin_confirm = u.is_admin.then_some("Revoke admin rights from this user?");
    let admin_reauth = (!u.is_admin && locked).then_some("grant admin rights");
    let disable_confirm =
        (!u.disabled).then_some("Disable this user? They will not be able to sign in.");
    let reset_reauth = locked.then_some("reset this user's password");
    html! {
        tr data-testid={"user-row-" (u.username)} {
            td { (u.username) }
            td {
                @if u.is_admin { span class="pill ok" data-testid="user-role" { "admin" } } @else { span class="pill start" data-testid="user-role" { "member" } }
                @if u.disabled { " " span class="pill fail" data-testid="user-disabled" { "disabled" } }
            }
            td {
                form class="inline" method="post" action={"/admin/users/" (u.id) "/password"} data-reauth=[reset_reauth] {
                    (csrf_field(csrf))
                    input type="password" name="password" placeholder="new password" autocomplete="new-password" data-testid="user-reset-input" required;
                    button class="btn" type="submit" data-testid="user-reset-submit" { "reset" }
                }
                @if u.is_self {
                    span class="btn disabled" title="You cannot change your own admin status" data-testid="user-toggle-admin" { (toggle_admin_label) }
                    span class="btn disabled" title="You cannot disable your own account" data-testid="user-toggle-disabled" { (toggle_disabled_label) }
                    span class="btn danger disabled" title="You cannot delete your own account" data-testid="user-delete" { "delete" }
                } @else {
                    form class="inline" method="post" action={"/admin/users/" (u.id) "/admin"} data-confirm=[admin_confirm] data-reauth=[admin_reauth] {
                        (csrf_field(csrf))
                        button class="btn" type="submit" data-testid="user-toggle-admin" { (toggle_admin_label) }
                    }
                    form class="inline" method="post" action={"/admin/users/" (u.id) "/disabled"} data-confirm=[disable_confirm] {
                        (csrf_field(csrf))
                        button class="btn" type="submit" data-testid="user-toggle-disabled" { (toggle_disabled_label) }
                    }
                    form class="inline" method="post" action={"/admin/users/" (u.id) "/delete"} data-confirm="Delete this user? This cannot be undone." {
                        (csrf_field(csrf))
                        button class="btn danger" type="submit" data-testid="user-delete" { "delete" }
                    }
                }
            }
        }
    }
}

fn add_user_form(view: &AdminView) -> Markup {
    html! {
        form method="post" action="/admin/users" data-reauth=[view.elevation_locked.then_some("create this user")] {
            (csrf_field(&view.csrf))
            div class="two" {
                div class="field" {
                    label for="username" { "Username" }
                    input id="username" name="username" autocomplete="off" data-testid="user-username-input" required;
                }
                div class="field" {
                    label for="password" { "Password" }
                    input id="password" name="password" type="password" autocomplete="new-password" data-testid="user-password-input" required;
                }
            }
            div class="two" {
                div class="field" {
                    label { "Role" }
                    label class="chk" {
                        input type="checkbox" name="is_admin" value="1" data-testid="user-admin-checkbox";
                        span class="nm" { "Admin" }
                    }
                }
            }
            div class="formactions" { button class="btn primary" type="submit" data-testid="user-submit" { "Create user" } }
        }
    }
}

fn projects_group(view: &AdminView) -> Markup {
    html! {
        div class="group" {
            div class="gh" {
                h2 { "All projects" }
                span class="count" { (view.projects.len()) " total" }
                span class="line" {}
            }
            p class="note mb-12" { "Every project across all users. Looking is not recorded; changes you make, and revealing a check's ping URL, are — see the audit trail below." }
            div class="list" {
                @if view.projects.is_empty() { p class="empty" { "No projects yet." } }
                @for (project, owner) in &view.projects {
                    div class="check" data-href={"/admin/projects/" (project.id)} {
                        div class="cmeta" {
                            div class="nm" { a href={"/admin/projects/" (project.id)} { (project.name) } }
                            div class="sc" { "owner: " (owner) }
                        }
                        a class="rowedit" href={"/admin/projects/" (project.id)} { "manage →" }
                    }
                }
            }
        }
    }
}
