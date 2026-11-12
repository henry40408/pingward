use maud::{DOCTYPE, Markup, html};

use crate::assets::{css_version, icon_version, js_version};

/// What the page chrome needs from the session.
pub struct Chrome<'a> {
    pub show_nav: bool,
    pub is_admin: bool,
    pub csrf: &'a str,
}

/// The shared document: head, optional nav bar, `body` inside `<main>`, footer.
// No `data-theme` on purpose: `theme-init.js` sets it, and without script its
// absence lets app.css's `prefers-color-scheme` block answer.
pub fn page(chrome: &Chrome<'_>, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "pingward" }
                link rel="stylesheet" href=(format!("/assets/app.css?v={}", css_version()));
                link rel="icon" type="image/svg+xml" href=(format!("/favicon.svg?v={}", icon_version()));
                link rel="apple-touch-icon" href=(format!("/apple-touch-icon.png?v={}", icon_version()));
                // Render-blocking on purpose (theme before first paint). Files, not inline
                // blocks, so the CSP can be `script-src 'self'`.
                script src=(format!("/assets/theme-init.js?v={}", js_version())) {}
                script src=(format!("/assets/app.js?v={}", js_version())) defer {}
            }
            body {
                @if chrome.show_nav { (nav(chrome)) }
                main class="wrap" { (body) }
                // Outside `show_nav` so login and setup show the version too.
                footer class="foot" {
                    span data-testid="app-version" { "pingward " (crate::view::version()) }
                }
            }
        }
    }
}

fn nav(chrome: &Chrome<'_>) -> Markup {
    html! {
        header class="bar" { div class="inner" {
            a class="brand" href="/" { span class="glyph" { "▚" } "pingward" }
            nav class="links" {
                a href="/" { "Dashboard" }
                a href="/account" data-testid="nav-account" { "Account" }
                @if chrome.is_admin { a href="/admin" data-testid="nav-admin" { "Admin" } }
            }
            div class="right" {
                button class="iconbtn" id="pw-theme-toggle" title="Toggle theme" aria-label="Toggle theme" { "◐" }
                form class="inline" method="post" action="/logout" {
                    (csrf_field(chrome.csrf))
                    button class="ghost" type="submit" data-testid="logout-button" { "Log out" }
                }
            }
        } }
    }
}

/// The hidden `_csrf` input every POST form carries.
pub fn csrf_field(csrf: &str) -> Markup {
    html! { input type="hidden" name="_csrf" value=(csrf); }
}
