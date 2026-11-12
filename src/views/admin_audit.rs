//! Audit-trail fragment, served by `GET /admin/audit` and composed into `/admin` as
//! `Markup` (see `check_history`). No hidden fields: nothing else on `/admin` reads
//! the query string.

use maud::{Markup, html};

use super::check_history::pager;

/// Four visible columns (when/actor/action/target); the rest of `models::AuditLog`
/// sits in an expandable row.
pub struct AuditRow {
    pub time: String,
    pub iso: String,
    pub actor: String,
    pub action: String,
    pub target: String,
    pub method_path: String,
    pub detail: String,
    pub target_owner: String,
    /// False when every expanded field is unset (no caret onto an empty box).
    pub expandable: bool,
}

pub struct AdminAuditView {
    pub rows: Vec<AuditRow>,
    /// Every actor/action present in the trail, for the two filter selects.
    pub actors: Vec<String>,
    pub actions: Vec<String>,
    /// `""` = all, echoed back into the controls.
    pub f_actor: String,
    pub f_action: String,
    pub f_from: String,
    pub f_to: String,
    /// Switches the empty state's wording and shows the Clear link.
    pub filtered: bool,
    pub newer: Option<String>,
    pub older: Option<String>,
}

pub fn admin_audit(view: &AdminAuditView) -> Markup {
    html! {
        form class="filters" method="get" action="/admin" data-testid="audit-filters" data-nosubmit {
            label class="fld" { "From " input type="datetime-local" name="afrom" data-testid="audit-from" data-utc=(view.f_from); }
            label class="fld" { "To " input type="datetime-local" name="ato" data-testid="audit-to" data-utc=(view.f_to); }
            label class="fld" { "Actor"
                select name="aactor" data-testid="audit-actor" {
                    option value="" selected[view.f_actor.is_empty()] { "All" }
                    @for a in &view.actors { option value=(a) selected[view.f_actor == *a] { (a) } }
                }
            }
            label class="fld" { "Action"
                select name="aaction" data-testid="audit-action" {
                    option value="" selected[view.f_action.is_empty()] { "All" }
                    @for a in &view.actions { option value=(a) selected[view.f_action == *a] { (a) } }
                }
            }
            button class="btn" type="submit" data-apply data-testid="audit-apply" { "Filter" }
            @if view.filtered { a class="btn ghost" href="/admin/audit" data-testid="audit-clear" { "Clear" } }
        }
        @if view.rows.is_empty() {
            p class="empty" data-testid="audit-empty" {
                @if view.filtered { "No audit entries match the filter." } @else { "No audit entries yet." }
            }
        } @else {
            div class="tscroll" {
                table {
                    thead { tr { th { "When" } th { "Actor" } th { "Action" } th { "Target" } } }
                    tbody {
                        @for r in &view.rows { (audit_row(r)) }
                    }
                }
            }
            (pager("audit", view.newer.as_deref(), view.older.as_deref(), "audit-pager"))
        }
    }
}

fn audit_row(r: &AuditRow) -> Markup {
    html! {
        tr class=[r.expandable.then_some("toggle")] data-testid="audit-row" {
            td class="mono" {
                span class={"caret" @if !r.expandable { " blank" }} { "▸" }
                span class="localtime" data-ts=(r.iso) { (r.time) }
            }
            td { (r.actor) }
            td class="mono" { (r.action) }
            td class="mono" { (r.target) }
        }
        @if r.expandable {
            tr class="exp" { td colspan="4" {
                // `.out` is `white-space: pre-wrap`: the newlines are the display.
                div class="out log" {
                    span class="tag" { "request · detail" }
                    (r.method_path) "\ntarget owner: " (r.target_owner)
                    @if !r.detail.is_empty() { "\n" (r.detail) }
                }
            } }
        }
    }
}
