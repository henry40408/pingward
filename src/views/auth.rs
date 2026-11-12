use maud::{Markup, html};

use super::layout::{Chrome, csrf_field, page};

pub struct AuthView {
    pub csrf: String,
    pub error: Option<String>,
}

fn chrome(view: &AuthView) -> Chrome<'_> {
    Chrome {
        show_nav: false,
        is_admin: false,
        csrf: &view.csrf,
    }
}

fn brand() -> Markup {
    html! { a class="brand" href="/" { span class="glyph" { "▚" } "pingward" } }
}

pub fn login(view: &AuthView) -> Markup {
    let body = html! {
        div class="auth" { div class="card" { div class="cb" {
            (brand())
            h1 { "Log in" }
            @if let Some(error) = &view.error {
                p class="flash err" data-testid="login-error" { (error) }
            }
            form method="post" action="/login" {
                (csrf_field(&view.csrf))
                div class="field" {
                    label for="username" { "Username" }
                    input id="username" name="username" autocomplete="username" data-testid="username-input" required;
                }
                div class="field" {
                    label for="password" { "Password" }
                    input id="password" name="password" type="password" autocomplete="current-password" data-testid="password-input" required;
                }
                button class="btn primary block" type="submit" data-testid="login-submit" { "Log in" }
            }
        } } }
    };
    page(&chrome(view), body)
}

pub fn setup(view: &AuthView) -> Markup {
    let body = html! {
        div class="auth" { div class="card" { div class="cb" {
            (brand())
            h1 { "Create the first admin" }
            @if let Some(error) = &view.error {
                p class="flash err" data-testid="setup-error" { (error) }
            }
            form method="post" action="/setup" {
                (csrf_field(&view.csrf))
                div class="field" {
                    label for="username" { "Username" }
                    input id="username" name="username" autocomplete="username" data-testid="username-input" required;
                }
                div class="field" {
                    label for="password" { "Password" }
                    input id="password" name="password" type="password" autocomplete="new-password" data-testid="password-input" required;
                }
                div class="field" {
                    label for="setup_token" { "Setup token" }
                    input id="setup_token" name="setup_token" autocomplete="off" autocapitalize="characters" spellcheck="false" placeholder="XXXXX-XXXXX" data-testid="setup-token-input" required;
                    div class="help" { "Printed in the server log (stderr) when pingward started with no users. Look for “Setup token”." }
                }
                button class="btn primary block" type="submit" data-testid="setup-submit" { "Create admin" }
            }
        } } }
    };
    page(&chrome(view), body)
}
