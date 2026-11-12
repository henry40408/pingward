use maud::{Markup, PreEscaped, html};

use super::layout::{Chrome, csrf_field, page};

pub struct AdminUnlockView {
    pub is_admin: bool,
    pub csrf: String,
    /// False for a passwordless forward-auth admin: the gate doesn't apply.
    pub applies: bool,
    /// `Some(readable duration)` when already confirmed.
    pub remaining: Option<String>,
    /// From `elevate::ELEVATION_TTL_SECS`, so the copy can't drift.
    pub ttl: String,
    /// Set when bounced here by a refused action.
    pub bounced_flash: Option<String>,
    pub error: Option<String>,
}

pub fn admin_unlock(view: &AdminUnlockView) -> Markup {
    let body = html! {
        div class="phead" { div {
            h1 { "Confirm it's you" }
            p class="crumb tight" { "Some admin actions ask for your password a second time, even though you're already signed in." }
        } }
        @if let Some(flash) = &view.bounced_flash {
            p class="flash err" data-testid="unlock-bounced" { (flash) }
        }
        @if let Some(error) = &view.error {
            p class="flash err" data-testid="unlock-error" { (error) }
        }
        @if !view.applies {
            (not_applicable())
        } @else if let Some(left) = &view.remaining {
            (already_confirmed(left))
        } @else {
            (password_form(view))
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

fn not_applicable() -> Markup {
    html! {
        div class="card" { div class="cb" {
            p class="note" data-testid="unlock-not-applicable" { "Your account signs in through your reverse proxy or SSO provider, so there is no password here to confirm. Every admin action is already available to you." }
            p { a class="btn" href="/admin" data-testid="unlock-back" { "Back to admin" } }
        } }
    }
}

fn already_confirmed(left: &str) -> Markup {
    html! {
        div class="card" { div class="cb" {
            p class="note" data-testid="unlock-state" { "Already confirmed " (PreEscaped("&mdash;")) " these actions stay available for another " (left) "." }
            p { a class="btn primary" href="/admin" data-testid="unlock-back" { "Back to admin" } }
        } }
    }
}

fn password_form(view: &AdminUnlockView) -> Markup {
    html! {
        div class="card" {
            div class="ch" { h2 { "Why you're seeing this" } }
            div class="cb" {
                p class="crumb tight" data-testid="unlock-gated" { "A session cookie proves who opened this browser, not who is at it now. " strong { "Creating a user" } ", " strong { "resetting another user's password" } " and " strong { "granting admin rights" } " each hand out access that keeps working after you sign out, so they ask for a second confirmation before they run." }
                p class="crumb tight" { "Actions that " em { "remove" } " access " (PreEscaped("&mdash;")) " disabling, demoting, deleting " (PreEscaped("&mdash;")) " never ask, so you can always shut an account down in a hurry." }
                p class="crumb tight" { "This is your same password again, not a second factor. Confirming lasts " (view.ttl) ", and ends when you sign out." }
            }
        }
        div class="card" {
            div class="ch" { h2 { "Confirm your password" } }
            div class="cb" {
                form method="post" action="/admin/unlock" {
                    (csrf_field(&view.csrf))
                    div class="field" {
                        label for="password" { "Password" }
                        input id="password" name="password" type="password" autocomplete="current-password" data-testid="unlock-input" required;
                    }
                    div class="formactions" {
                        button class="btn primary" type="submit" data-testid="unlock-submit" { "Confirm" }
                        a class="btn" href="/admin" data-testid="unlock-cancel" { "Cancel" }
                    }
                }
            }
        }
    }
}
