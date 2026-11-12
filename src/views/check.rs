use maud::{Markup, PreEscaped, html};

use super::layout::{Chrome, csrf_field, page};
use crate::models::Check;

pub struct ChannelBox {
    pub id: i64,
    pub name: String,
    pub kind: &'static str,
    pub bound: bool,
}

pub struct CheckView {
    pub csrf: String,
    pub is_admin: bool,
    pub admin: bool,
    pub check: Check,
    pub description_html: String,
    pub project_name: String,
    pub status: &'static str,
    pub since: String,
    pub next_due: crate::view::NextDue,
    pub schedule: String,
    pub ping_url: String,
    /// Withheld pending an audited reveal (see `web::CheckPageViewer`).
    pub ping_url_hidden: bool,
    pub bars: Vec<crate::view::Bar>,
    pub channel_boxes: Vec<ChannelBox>,
    /// The same fragments the history endpoints serve, composed as `Markup`.
    pub pings_partial: Markup,
    pub notifs_partial: Markup,
    pub flash: Option<String>,
}

pub fn check(view: &CheckView) -> Markup {
    let base = if view.admin { "/admin" } else { "" };
    let id = view.check.id;
    let body = html! {
        @if let Some(flash) = &view.flash { p class="flash ok" data-testid="check-flash" { (flash) } }
        div class="crumb" {
            a href="/" { "Checks" } span class="sep" { "/" }
            a href={(base) "/projects/" (view.check.project_id)} { (view.project_name) } span class="sep" { "/" } (view.check.name)
        }
        (header(view))
        div class="crumb tight" { (view.schedule) }
        @if !view.description_html.is_empty() {
            // `markdown::render` escapes first, then re-adds a whitelist of tags.
            div class="desc" data-testid="check-description" { (PreEscaped(&view.description_html)) }
        }
        (actions(view, base))
        (ping_card(view, base))
        (channels_card(view, base))
        div class="card" id="pings-card" {
            div class="ch" {
                h2 { "Recent pings" }
                button class="btn live-toggle" type="button" id="pings-live" data-testid="pings-live" aria-pressed="false" data-endpoint={(base) "/checks/" (id) "/events"} { "LIVE" }
            }
            div class="cb" id="pings-section" data-endpoint={(base) "/checks/" (id) "/pings"} { (view.pings_partial) }
        }
        div class="card flush" {
            div class="ch" { h2 { "Recent notifications" } }
            div class="cb" id="notifs-section" data-endpoint={(base) "/checks/" (id) "/notifications"} { (view.notifs_partial) }
        }
    };
    page(
        &Chrome {
            show_nav: true,
            is_admin: view.is_admin,
            csrf: &view.csrf,
        },
        body,
    )
}

fn header(view: &CheckView) -> Markup {
    html! {
        div class="chead" {
            span class={"status-dot " (view.status)} {}
            h1 { (view.check.name) }
            span class={"badge " (view.status)} data-testid="check-status" { (view.status) }
            span class="cmeta" {
                span class={"since " (view.status)} { (view.since) }
                span class="due" data-testid="check-next-due" title=[view.next_due.iso.as_deref()] { (view.next_due.label) }
            }
        }
    }
}

fn actions(view: &CheckView, base: &str) -> Markup {
    let id = view.check.id;
    let csrf = &view.csrf;
    html! {
        div class="actions" {
            a class="btn" href={(base) "/checks/" (id) "/edit"} { "Edit" }
            @if view.check.status.as_str() == "paused" {
                form class="inline" method="post" action={(base) "/checks/" (id) "/resume"} { (csrf_field(csrf)) button class="btn" data-testid="resume-button" { "Resume" } }
            } @else {
                form class="inline" method="post" action={(base) "/checks/" (id) "/pause"} { (csrf_field(csrf)) button class="btn" data-testid="pause-button" { "Pause" } }
            }
            @if view.status == "down" && !view.check.acknowledged {
                form class="inline" method="post" action={(base) "/checks/" (id) "/ack"} { (csrf_field(csrf)) button class="btn primary" data-testid="ack-button" { "Acknowledge" } }
            }
            form class="inline" method="post" action={(base) "/checks/" (id) "/regenerate"}
                data-confirm="Regenerate the ping URL? The current URL stops working immediately and any job using it must be updated." {
                (csrf_field(csrf)) button class="btn danger" data-testid="regenerate-button" { "Regenerate URL" }
            }
            form class="inline" method="post" action={(base) "/checks/" (id) "/delete"} data-confirm="Delete this check?" {
                (csrf_field(csrf)) button class="btn danger" data-testid="delete-check-button" { "Delete" }
            }
        }
    }
}

fn ping_card(view: &CheckView, base: &str) -> Markup {
    html! {
        div class="card" { div class="cb" {
            @if view.ping_url_hidden {
                (withheld_url(view, base))
            } @else {
                (ping_url_help(&view.ping_url))
            }
            // No run count: only the browser knows how many bars are visible.
            div class="beat" title="most recent runs, newest on the right" {
                @for b in &view.bars {
                    i class=(b.class) style={"height:" (b.height) "px"} title=(b.title) {}
                }
            }
            // "taller = longer" holds only for solid bars; hollow ones are full height.
            div class="beatcap" { span { "older" } span class="key" { "solid = timed run, taller = longer · hollow = duration not measurable · amber ≥80% budget · red = failed" } span { "now" } }
        } }
    }
}

/// Cross-user admin view: the URL is a credential, so it (and the usage help
/// repeating it) stays hidden until revealed.
fn withheld_url(view: &CheckView, base: &str) -> Markup {
    html! {
        div class="urlrow withheld" data-testid="ping-url-withheld" {
            span class="note" { "Ping URL hidden. It is a credential — anyone holding it can mark this check up or down." }
            form class="inline" method="post" action={(base) "/checks/" (view.check.id) "/ping-url"} {
                (csrf_field(&view.csrf))
                button class="btn" type="submit" data-testid="reveal-ping-url" { "Reveal ping URL" }
            }
        }
        p class="note" data-testid="ping-url-reveal-note" { "Revealing it is recorded in the audit trail, with your username and this check." }
    }
}

fn ping_url_help(url: &str) -> Markup {
    html! {
        div class="urlrow" { code data-testid="ping-url" { (url) } button class="copy" type="button" data-copy=(url) { "Copy" } }
        details class="pinghelp" data-testid="ping-help" {
            summary { "How do I ping this check?" }
            div class="pinghelp-body" {
                p { "Have your job call the URL above when it runs — a plain " code { "GET" } " or " code { "POST" } " reports " strong { "success" } ". Add a suffix for other signals:" }
                div class="tscroll" { table {
                    thead { tr { th { "Signal" } th { "URL" } th { "Effect" } } }
                    tbody {
                        tr { td { span class="pill ok" { "success" } } td class="mono" { (url) } td { "marks the check up" } }
                        tr { td { span class="pill fail" { "fail" } } td class="mono" { (url) "/fail" } td { "marks the check down" } }
                        tr { td { span class="pill start" { "start" } } td class="mono" { (url) "/start" } td { "opens a run so an overrun can be caught" } }
                        tr { td { span class="pill log" { "log" } } td class="mono" { (url) "/log" } td { "records a note, status unchanged" } }
                        tr { td class="mono" { "exit code" } td class="mono" { (url) "/<code>" } td { code { "0" } " = up, anything else = down" } }
                    }
                } }
                // `.out` is `white-space: pre-wrap`: the newlines below are the display.
                div class="out log pinghelp-code" {
                    span class="tag" { "shell examples" }
                    "# report success when the job finishes\ncurl -fsS " (url)
                    "\n\n# wrap a job: start it, run, then report the exit code\ncurl -fsS " (url)
                    "/start\nmybackup.sh\ncurl -fsS " (url)
                    "/$?\n\n# report failure and attach the output (shown in Recent pings)\nmybackup.sh 2>&1 | curl -fsS --data-binary @- " (url) "/fail"
                }
            }
        }
    }
}

fn channels_card(view: &CheckView, base: &str) -> Markup {
    let id = view.check.id;
    html! {
        div class="card" {
            div class="ch" { h2 { "Notify channels" } }
            div class="cb" {
                @if view.channel_boxes.is_empty() {
                    p class="note" data-testid="check-channels-empty" { "No channels yet. " a href={(base) "/projects/" (view.check.project_id) "/channels/new"} { "Add one" } " to get notified when this check goes down." }
                } @else {
                    form method="post" action={(base) "/checks/" (id) "/channels"} {
                        (csrf_field(&view.csrf))
                        @for cb in &view.channel_boxes {
                            label class="chk" {
                                input type="checkbox" name="channel_ids" value=(cb.id) checked[cb.bound];
                                span class="nm" { (cb.name) }
                                span class="kind" { (cb.kind) }
                                span class="onoff" data-testid={"channel-state-" (cb.id)} { span class="on" { "ON" } span class="off" { "OFF" } }
                            }
                        }
                        div class="formactions" { button class="btn primary" type="submit" { "Save channels" } }
                    }
                }
            }
        }
    }
}
