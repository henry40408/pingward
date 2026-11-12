use maud::{Markup, html};

use super::layout::{Chrome, csrf_field, page};

pub struct CheckFormView {
    pub csrf: String,
    pub is_admin: bool,
    pub heading: String,
    pub action: String,
    pub error: Option<String>,
    pub name: String,
    pub description: String,
    pub schedule_kind: String,
    pub period_secs: String,
    pub cron_expr: String,
    pub grace_secs: String,
    pub timezone: String,
    pub scan_interval_secs: String,
    pub max_runtime_secs: String,
    pub nag_interval_secs: String,
}

pub fn check_form(view: &CheckFormView) -> Markup {
    let kind = view.schedule_kind.as_str();
    let body = html! {
        h1 { (view.heading) }
        @if let Some(error) = &view.error { p class="flash err" { (error) } }
        div class="card" { div class="cb" {
            form method="post" action=(view.action) {
                (csrf_field(&view.csrf))
                // Suggestions for the five duration fields below; the server still validates.
                datalist id="dur-list" {
                    @for d in crate::view::durations() { option value=(d) {} }
                }
                div class="field" {
                    label for="name" { "Name" }
                    input id="name" name="name" value=(view.name) data-testid="check-name-input" required;
                }
                div class="field" {
                    label for="description" { "Description" }
                    textarea id="description" name="description" rows="4" data-testid="check-description-input" { (view.description) }
                    div class="help" { "Optional. Supports **bold**, *italic*, `code`, [links](url) and - lists." }
                }
                div class="field" {
                    label for="schedule_kind" { "Schedule kind" }
                    select id="schedule_kind" name="schedule_kind" {
                        option value="period" selected[kind == "period"] { "period" }
                        option value="cron" selected[kind == "cron"] { "cron" }
                    }
                }
                div class="sched" data-sched="period" {
                    div class="field" {
                        label for="period_secs" { "Period" }
                        input id="period_secs" name="period_secs" value=(view.period_secs) list="dur-list" autocomplete="off" data-testid="check-period-input";
                        div class="help" { "Seconds, or a duration like 5m, 1h30m, 2d." }
                    }
                }
                div class="sched" data-sched="cron" {
                    div class="field" {
                        label for="cron_expr" { "Cron expression" }
                        input id="cron_expr" name="cron_expr" value=(view.cron_expr);
                        div class="help" { "6-field \"sec min hour dom mon dow\"." }
                    }
                }
                div class="two" {
                    div class="field" {
                        label for="grace_secs" { "Grace" }
                        input id="grace_secs" name="grace_secs" value=(view.grace_secs) list="dur-list" autocomplete="off" required;
                        div class="help" { "Seconds, or a duration like 5m, 1h30m." }
                    }
                    div class="field" {
                        label for="timezone" { "Timezone" }
                        // A datalist, not a <select>, for type-to-filter; `web::validate_timezone`
                        // checks it either way.
                        input id="timezone" name="timezone" value=(view.timezone) list="tz-list" autocomplete="off" required;
                        datalist id="tz-list" {
                            @for tz in crate::view::timezones() { option value=(tz) {} }
                        }
                        div class="help" { "IANA name. Only cron schedules use it to pick the firing wall clock." }
                    }
                }
                div class="field" {
                    label for="scan_interval_secs" { "Scan interval" }
                    input id="scan_interval_secs" name="scan_interval_secs" value=(view.scan_interval_secs) list="dur-list" autocomplete="off";
                    div class="help" { "Leave blank to inherit from the project. Seconds, or a duration like 5m, 1h30m." }
                }
                div class="field" {
                    label for="max_runtime_secs" { "Max runtime" }
                    input id="max_runtime_secs" name="max_runtime_secs" value=(view.max_runtime_secs) list="dur-list" autocomplete="off";
                    div class="help" { "Leave blank to turn off the runtime budget. Seconds, or a duration like 5m, 1h30m." }
                }
                div class="field" {
                    label for="nag_interval_secs" { "Nag interval" }
                    input id="nag_interval_secs" name="nag_interval_secs" value=(view.nag_interval_secs) list="dur-list" autocomplete="off";
                    div class="help" { "Leave blank to inherit from the project, or to turn off nagging. Seconds, or a duration like 5m, 1h30m." }
                }
                div class="btnrow" {
                    button class="btn primary" type="submit" data-testid="check-submit" { "Save changes" }
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
