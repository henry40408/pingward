//! The two history fragments of the check page. Served standalone (JS swaps them
//! into `#pings-section` / `#notifs-section`) and composed into the page as
//! `Markup`, so page load and refresh share one source and nothing needs `|safe`.
//!
//! Each is a real GET form to the check page, so it filters without script; with
//! script, `app.js` swaps the fragment instead. The hidden fields carry the
//! sibling section's filter, since a GET submit replaces the whole query.
//! `datetime-local` is local time with JS (converted before fetch) and read as
//! UTC without (`web::parse_date_bound`), matching what each mode displays.

use maud::{Markup, html};

pub struct HiddenField {
    pub name: &'static str,
    pub value: String,
}

pub struct PingRow {
    pub time: String,             // UTC fallback shown when JS is off
    pub iso: String,              // RFC3339 UTC; localized to the viewer's zone client-side
    pub pill_class: &'static str, // "ok"|"fail"|"start"|"log"
    pub kind_label: &'static str, // "success"|"fail"|"start"|"log"
    pub exit: String,
    pub duration: String,
    pub source: String,
    pub body: String,
}

pub struct NotificationRow {
    pub created_at: String,             // UTC fallback shown when JS is off
    pub iso: String,                    // RFC3339 UTC; localized client-side
    pub event: &'static str,            // "down"|"up"|"reminder"
    pub event_pill_class: &'static str, // mirrors the ping-kind pills
    pub status: &'static str,
    pub channel: String,
    pub error: String,
}

/// `base` is `""` or `/admin`.
pub struct CheckPingsView {
    pub base: String,
    pub check_id: i64,
    pub rows: Vec<PingRow>,
    /// `""` = all, canonicalized from the query.
    pub f_kind: String,
    /// `Z`-form RFC3339 UTC (`""` = unset); localized client-side.
    pub f_from: String,
    pub f_to: String,
    /// Controls the "Clear" affordance.
    pub filtered: bool,
    /// The notifications filter, re-sent so a scriptless submit here keeps it.
    pub carry: Vec<HiddenField>,
    /// Clear link; keeps the other section's filter.
    pub clear: String,
    pub newer: Option<String>,
    pub older: Option<String>,
}

/// Otherwise as [`CheckPingsView`].
pub struct CheckNotifsView {
    pub base: String,
    pub check_id: i64,
    pub rows: Vec<NotificationRow>,
    /// `""` = all: up|down|reminder.
    pub f_event: String,
    /// `""` = all: ok|error.
    pub f_status: String,
    pub f_from: String,
    pub f_to: String,
    pub filtered: bool,
    /// The pings section's filter.
    pub carry: Vec<HiddenField>,
    pub clear: String,
    pub newer: Option<String>,
    pub older: Option<String>,
}

fn carry_inputs(carry: &[HiddenField]) -> Markup {
    html! {
        @for f in carry { input type="hidden" name=(f.name) value=(f.value); }
    }
}

fn pager(
    testid_prefix: &str,
    newer: &Option<String>,
    older: &Option<String>,
    pager_id: &str,
) -> Markup {
    let link = |href: &Option<String>, suffix: &str, label: &str| {
        let testid = format!("{testid_prefix}-{suffix}");
        html! {
            @if let Some(href) = href {
                a class="btn" href=(href) data-testid=(testid) { (label) }
            } @else {
                span class="btn disabled" aria-disabled="true" data-testid=(testid) { (label) }
            }
        }
    };
    html! {
        div class="pager" data-testid=(pager_id) {
            (link(newer, "newer", "← Newer"))
            (link(older, "older", "Older →"))
        }
    }
}

pub fn check_pings(view: &CheckPingsView) -> Markup {
    let kind = view.f_kind.as_str();
    html! {
        form class="filters" method="get" action={(view.base) "/checks/" (view.check_id)} data-testid="pings-filters" data-nosubmit {
            (carry_inputs(&view.carry))
            label class="fld" { "From " input type="datetime-local" name="pfrom" data-testid="pings-from" data-utc=(view.f_from); }
            label class="fld" { "To " input type="datetime-local" name="pto" data-testid="pings-to" data-utc=(view.f_to); }
            label class="fld" { "Kind"
                select name="pk" data-testid="pings-kind" {
                    option value="" selected[kind.is_empty()] { "All" }
                    option value="success" selected[kind == "success"] { "success" }
                    option value="fail" selected[kind == "fail"] { "fail" }
                    option value="start" selected[kind == "start"] { "start" }
                    option value="log" selected[kind == "log"] { "log" }
                }
            }
            button class="btn" type="submit" data-apply data-testid="pings-apply" { "Filter" }
            @if view.filtered { a class="btn ghost" href=(view.clear) data-testid="pings-clear" { "Clear" } }
        }
        @if view.rows.is_empty() {
            p class="empty" data-testid="pings-empty" {
                @if view.filtered { "No pings match the filter." } @else { "No pings yet." }
            }
        } @else {
            div class="tscroll" {
                table {
                    thead { tr { th { "When" } th { "Kind" } th { "Exit" } th { "Duration" } th { "Source" } } }
                    tbody {
                        @for p in &view.rows { (ping_row(p)) }
                    }
                }
            }
            (pager("pings", &view.newer, &view.older, "pings-pager"))
        }
    }
}

fn ping_row(p: &PingRow) -> Markup {
    let has_body = !p.body.is_empty();
    html! {
        tr class=[has_body.then_some("toggle")] data-testid="ping-row" {
            td class="mono" {
                span class={"caret" @if !has_body { " blank" }} { "▸" }
                span class="localtime" data-ts=(p.iso) { (p.time) }
            }
            td { span class={"pill " (p.pill_class)} { (p.kind_label) } }
            td class="mono" { (p.exit) }
            td class="mono" { (p.duration) }
            td class="mono" { (p.source) }
        }
        @if has_body {
            tr class="exp" { td colspan="5" {
                // `.out` is `white-space: pre-wrap`: the body must follow the tag with no
                // whitespace between them.
                div class={"out " (p.pill_class)} { span class="tag" { "captured output · POST body" } (p.body) }
            } }
        }
    }
}

pub fn check_notifs(view: &CheckNotifsView) -> Markup {
    let event = view.f_event.as_str();
    let status = view.f_status.as_str();
    html! {
        form class="filters" method="get" action={(view.base) "/checks/" (view.check_id)} data-testid="notifs-filters" data-nosubmit {
            (carry_inputs(&view.carry))
            label class="fld" { "From " input type="datetime-local" name="nfrom" data-testid="notifs-from" data-utc=(view.f_from); }
            label class="fld" { "To " input type="datetime-local" name="nto" data-testid="notifs-to" data-utc=(view.f_to); }
            label class="fld" { "Event"
                select name="ne" data-testid="notifs-event" {
                    option value="" selected[event.is_empty()] { "All" }
                    option value="down" selected[event == "down"] { "down" }
                    option value="up" selected[event == "up"] { "up" }
                    option value="reminder" selected[event == "reminder"] { "reminder" }
                }
            }
            label class="fld" { "Result"
                select name="ns" data-testid="notifs-status" {
                    option value="" selected[status.is_empty()] { "All" }
                    option value="ok" selected[status == "ok"] { "sent" }
                    option value="error" selected[status == "error"] { "error" }
                }
            }
            button class="btn" type="submit" data-apply data-testid="notifs-apply" { "Filter" }
            @if view.filtered { a class="btn ghost" href=(view.clear) data-testid="notifs-clear" { "Clear" } }
        }
        @if view.rows.is_empty() {
            p class="empty" data-testid="notifications-empty" {
                @if view.filtered { "No notifications match the filter." } @else { "No notifications yet." }
            }
        } @else {
            div class="tscroll" {
                table {
                    thead { tr { th { "When" } th { "Event" } th { "Channel" } th { "Result" } } }
                    tbody {
                        @for n in &view.rows {
                            tr {
                                td class="mono" { span class="localtime" data-ts=(n.iso) { (n.created_at) } }
                                td { span class={"pill " (n.event_pill_class)} { (n.event) } }
                                td { (n.channel) }
                                @if n.status == "ok" {
                                    td class="mono ok-txt" { "sent" }
                                } @else {
                                    td class="mono fail-txt" { (n.error) }
                                }
                            }
                        }
                    }
                }
            }
            (pager("notif", &view.newer, &view.older, "notif-pager"))
        }
    }
}
