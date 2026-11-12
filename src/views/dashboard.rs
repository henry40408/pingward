use maud::{Markup, html};

use super::layout::{Chrome, page};

pub struct DashboardView {
    pub csrf: String,
    pub is_admin: bool,
    pub total: usize,
    /// Includes `Running`: an in-flight run is still up, so the two share a tile.
    pub up: usize,
    pub late: usize,
    pub down: usize,
    pub groups: Vec<ProjectGroup>,
    pub q: String,
    pub status: String,
    /// Forward-auth logout with no gateway URL: tell the visitor to sign out at the proxy.
    pub forward_auth_logout: Option<String>,
}

pub struct CheckRow {
    pub id: i64,
    pub name: String,
    pub status: &'static str,
    pub schedule: String, // e.g. "every 1h · 10m grace" or the cron expr
    pub last: String,
    pub bars: Vec<crate::view::Bar>,
    pub description: String, // markdown::truncate_plain, single-line summary
    /// No bound channels: rendered as a chip, so a check nobody is alerted for shows.
    pub no_channel: bool,
}

pub struct ProjectGroup {
    pub id: i64,
    pub name: String,
    pub count: usize,
    pub checks: Vec<CheckRow>,
    pub description: String, // markdown::truncate_plain, single-line summary
}

pub fn dashboard(view: &DashboardView) -> Markup {
    let filtered = !view.q.is_empty() || !view.status.is_empty();
    let body = html! {
        @if let Some(msg) = &view.forward_auth_logout {
            p class="flash warn" data-testid="forward-auth-logout-flash" { (msg) }
        }
        div class="phead" {
            div { h1 { "Checks" } }
            a class="btn-primary" href="/projects/new" { "New project" }
        }
        (filter_form(view, filtered))
        (tiles(view))
        @if view.groups.is_empty() {
            (empty_list(filtered))
        } @else {
            @for group in &view.groups { (group_card(group)) }
            (legend())
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

fn filter_form(view: &DashboardView, filtered: bool) -> Markup {
    let status = view.status.as_str();
    html! {
        form class="filters" method="get" action="/" role="search" {
            label class="fld" { "Filter"
                input type="search" name="q" value=(view.q) placeholder="project or check name / description" data-testid="dashboard-filter-input";
            }
            label class="fld" { "Status"
                select name="status" data-testid="dashboard-status-filter" {
                    option value="" selected[status.is_empty()] { "All" }
                    option value="up" selected[status == "up"] { "Up" }
                    option value="late" selected[status == "late"] { "Late" }
                    option value="down" selected[status == "down"] { "Down" }
                }
            }
            button class="btn" type="submit" data-testid="dashboard-filter-submit" { "Filter" }
            @if filtered {
                a class="btn" href="/" data-testid="dashboard-filter-clear" { "Clear" }
            }
        }
    }
}

fn tiles(view: &DashboardView) -> Markup {
    let tile = |class: &str, n: usize, label: &str| {
        html! {
            div class={"tile " (class)} { span class="edge" {} div class="n" { (n) } div class="l" { (label) } }
        }
    };
    html! {
        div class="tiles" {
            (tile("total", view.total, "Total"))
            (tile("up", view.up, "Up"))
            (tile("late", view.late, "Late"))
            (tile("down", view.down, "Down"))
        }
    }
}

fn empty_list(filtered: bool) -> Markup {
    html! {
        div class="list" {
            @if filtered {
                p class="empty" data-testid="dashboard-no-results" { "No checks match the current filter. " a href="/" { "Clear the filter" } " to see every check." }
            } @else {
                p class="empty" data-testid="dashboard-empty" { "No projects yet. Create one to start watching a job." }
            }
        }
    }
}

fn group_card(group: &ProjectGroup) -> Markup {
    html! {
        div class="group" {
            div class="gh" {
                h2 { (group.name) }
                @if !group.description.is_empty() { span class="gdesc" { (group.description) } }
                span class="count" { (group.count) " checks" }
                span class="line" {}
                a href={"/projects/" (group.id)} { "Manage →" }
            }
            div class="list" {
                @for check in &group.checks { (check_row(check)) }
            }
        }
    }
}

fn check_row(check: &CheckRow) -> Markup {
    html! {
        div class="check" data-testid="dashboard-check-row" data-href={"/checks/" (check.id)} {
            span class={"status-dot " (check.status)} {}
            div class="cmeta" {
                div class="nm" { a href={"/checks/" (check.id)} { (check.name) } }
                div class="sc" { (check.schedule) }
                @if !check.description.is_empty() {
                    div class="cdesc" data-testid="check-description-summary" { (check.description) }
                }
            }
            @if check.no_channel { (super::project::no_channel_chip()) }
            div class="spark" {
                @for bar in &check.bars {
                    i class=(bar.class) style={"height:" (bar.height) "px"} title=(bar.title) {}
                }
            }
            div class="cwhen" { (check.last) }
            span class={"badge " (check.status)} { (check.status) }
        }
    }
}

fn legend() -> Markup {
    let key = |swatch: &str, text: &str| {
        html! { span class="k" { span class="mini" { i class=(swatch) {} } (text) } }
    };
    html! {
        div class="legend" {
            span class="legendhead" { "Heartbeat" }
            (key("sw-short", "short = fast run · lots of headroom"))
            (key("sw-tall", "tall = long run · nearing timeout"))
            (key("sw-late", "amber = used ≥80% of budget"))
            (key("sw-down", "red = failed"))
            // "not measurable": a timed run with no ceiling is hollow too (view::heartbeat).
            (key("sw-none", "hollow = success, duration not measurable"))
            (key("sw-paused", "flat = paused"))
        }
    }
}
