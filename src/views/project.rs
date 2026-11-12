use maud::{Markup, PreEscaped, html};

use super::layout::{Chrome, csrf_field, page};
use crate::models::Project;

pub struct ProjectFormView {
    pub csrf: String,
    pub is_admin: bool,
    pub heading: String,
    pub action: String,
    pub name: String,
    pub description: String,
    pub scan_interval_secs: String,
    pub nag_interval_secs: String,
    pub error: Option<String>,
}

pub struct ProjectView {
    pub csrf: String,
    pub is_admin: bool,
    pub admin: bool,
    pub project: Project,
    pub description_html: String,
    pub checks: Vec<ProjectCheckRow>,
    pub channels: Vec<ProjectChannelRow>,
    pub test_result: Option<TestResult>,
}

/// `status` is precomputed: it needs `now`.
pub struct ProjectCheckRow {
    pub id: i64,
    pub name: String,
    pub status: &'static str,
    pub schedule: String,
    pub description: String, // markdown::truncate_plain, single-line summary
    /// No bound channels (rendered as a chip).
    pub no_channel: bool,
}

/// Projected so a `Channel`'s `config_json` (delivery secrets) never reaches the
/// view, as with `ChannelEditView`.
pub struct ProjectChannelRow {
    pub id: i64,
    pub name: String,
    pub kind: &'static str,
}

pub struct TestResult {
    pub ok: bool,
    pub message: String,
}

pub fn no_channel_chip() -> Markup {
    html! {
        span class="nochan" data-testid="check-no-channel" title="No notification channel is bound — nobody is alerted when this check goes down" { "no channel" }
    }
}

pub fn project_form(view: &ProjectFormView) -> Markup {
    let body = html! {
        h1 { (view.heading) }
        @if let Some(error) = &view.error { p class="flash err" { (error) } }
        div class="card" { div class="cb" {
            form method="post" action=(view.action) {
                (csrf_field(&view.csrf))
                // See `view::durations`.
                datalist id="dur-list" {
                    @for d in crate::view::durations() { option value=(d) {} }
                }
                div class="field" {
                    label for="name" { "Name" }
                    input id="name" name="name" value=(view.name) data-testid="project-name-input" required;
                }
                div class="field" {
                    label for="description" { "Description" }
                    textarea id="description" name="description" rows="4" data-testid="project-description-input" { (view.description) }
                    div class="help" { "Optional. Supports **bold**, *italic*, `code`, [links](url) and - lists." }
                }
                div class="field" {
                    label for="scan_interval_secs" { "Scan interval" }
                    input id="scan_interval_secs" name="scan_interval_secs" value=(view.scan_interval_secs) list="dur-list" autocomplete="off";
                    div class="help" { "Leave blank to inherit the default scan interval. Seconds, or a duration like 5m, 1h30m." }
                }
                div class="field" {
                    label for="nag_interval_secs" { "Nag interval" }
                    input id="nag_interval_secs" name="nag_interval_secs" value=(view.nag_interval_secs) list="dur-list" autocomplete="off";
                    div class="help" { "Leave blank to inherit, or to turn off nagging. Seconds, or a duration like 5m, 1h30m." }
                }
                div class="btnrow" {
                    button class="btn primary" type="submit" data-testid="project-submit" { "Save changes" }
                }
            }
        } }
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

pub fn project(view: &ProjectView) -> Markup {
    let base = if view.admin { "/admin" } else { "" };
    let id = view.project.id;
    let body = html! {
        div class="phead" { div { h1 { (view.project.name) } } }
        @if !view.description_html.is_empty() {
            // `markdown::render` escapes first, then re-adds a whitelist of tags.
            div class="desc" data-testid="project-description" { (PreEscaped(&view.description_html)) }
        }
        div class="actions" {
            a class="btn" href={(base) "/projects/" (id) "/edit"} { "Edit" }
            a class="btn" href={(base) "/projects/" (id) "/checks/new"} data-testid="new-check-link" { "New check" }
            a class="btn" href={(base) "/projects/" (id) "/channels/new"} { "New channel" }
            form class="inline" method="post" action={(base) "/projects/" (id) "/delete"} data-confirm="Delete project?" {
                (csrf_field(&view.csrf))
                button class="btn danger" type="submit" data-testid="delete-project-button" { "Delete project" }
            }
        }
        @if let Some(result) = &view.test_result {
            p class={"flash " (if result.ok { "ok" } else { "err" })} { (result.message) }
        }
        (checks_group(view, base))
        (channels_card(view, base))
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

fn checks_group(view: &ProjectView, base: &str) -> Markup {
    html! {
        div class="group" {
            div class="gh" { h2 { "Checks" } span class="line" {} }
            div class="list" {
                @if view.checks.is_empty() {
                    p class="empty" data-testid="checks-empty" { "No checks yet." }
                }
                @for check in &view.checks {
                    div class="check" data-href={(base) "/checks/" (check.id)} {
                        span class={"status-dot " (check.status)} {}
                        div class="cmeta" {
                            div class="nm" { a href={(base) "/checks/" (check.id)} { (check.name) } }
                            div class="sc" { (check.schedule) }
                            @if !check.description.is_empty() {
                                div class="cdesc" data-testid="check-description-summary" { (check.description) }
                            }
                        }
                        @if check.no_channel { (no_channel_chip()) }
                        a class="rowedit" href={(base) "/checks/" (check.id) "/edit"} { "edit" }
                        span class={"badge " (check.status)} { (check.status) }
                    }
                }
            }
        }
    }
}

fn channels_card(view: &ProjectView, base: &str) -> Markup {
    html! {
        div class="card flush" {
            div class="ch" { h2 { "Channels" } }
            div class="cb" {
                @if view.channels.is_empty() {
                    p class="flash warn" data-testid="project-channels-empty" { "No channels yet — when a check in this project goes down, nobody is notified. " a href={(base) "/projects/" (view.project.id) "/channels/new"} { "Add a channel" } "." }
                }
                @for ch in &view.channels {
                    div class="chk readonly" {
                        span class="nm" { (ch.name) }
                        span class="kind" { (ch.kind) }
                        a class="btn ml-8" href={(base) "/channels/" (ch.id) "/edit"} data-testid={"channel-edit-" (ch.id)} { "edit" }
                        form class="inline" method="post" action={(base) "/channels/" (ch.id) "/test"} {
                            (csrf_field(&view.csrf))
                            button class="btn" type="submit" { "Send test" }
                        }
                        form class="inline" method="post" action={(base) "/channels/" (ch.id) "/delete"} data-confirm="Delete channel?" {
                            (csrf_field(&view.csrf))
                            button class="btn danger" type="submit" { "delete" }
                        }
                    }
                }
            }
        }
    }
}
