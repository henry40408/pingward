use maud::{Markup, html};

use super::layout::{Chrome, csrf_field, page};

pub struct ConfirmView {
    pub is_admin: bool,
    pub csrf: String,
    pub title: &'static str,
    pub message: &'static str,
    pub button: &'static str,
    pub action: String,
    pub cancel: String,
}

/// The no-script stand-in for `app.js`'s `confirm()`: a destructive POST
/// without `?confirmed=1` lands here, and the form re-posts it with the flag.
pub fn confirm(view: &ConfirmView) -> Markup {
    let body = html! {
        div class="phead" { div {
            h1 { (view.title) }
            p class="crumb tight" data-testid="confirm-why" { "Nothing has happened yet. This page is standing in for the confirmation prompt your browser would normally show, so the action is not carried out until you say so here." }
        } }
        div class="card" { div class="cb" {
            p class="crumb tight" data-testid="confirm-message" { (view.message) }
            form method="post" action=(view.action) {
                (csrf_field(&view.csrf))
                div class="formactions" {
                    button class="btn danger" type="submit" data-testid="confirm-submit" { (view.button) }
                    a class="btn" href=(view.cancel) data-testid="confirm-cancel" { "Cancel" }
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
