use chrono::{DateTime, Utc};
use maud::{Markup, html};

use super::layout::{Chrome, csrf_field, page};

pub struct AccountView {
    pub csrf: String,
    pub is_admin: bool,
    // sessions section
    pub sessions: Vec<SessionRow>,
    /// Hides the "revoke others" control when there is nothing else to revoke.
    pub other_count: usize,
    /// False for a passwordless forward-auth account: hides the password card and the
    /// API-key re-auth field (`web::reauthenticate` passes it unchallenged).
    pub has_password: bool,
    // password section
    pub password_error: Option<String>,
    pub password_flash: Option<String>,
    // api-keys section
    pub keys: Vec<ApiKeyRow>,
    /// Rendered exactly once, right after creation; never recoverable after.
    pub new_token: Option<String>,
    pub key_error: Option<String>,
}

/// `models::Session` minus the raw `id`, plus `handle` and `current`.
pub struct SessionRow {
    pub handle: String,
    pub created_at: Option<DateTime<Utc>>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub current: bool,
    pub sso: bool,
}

/// An expired key still lists so it can be revoked, hence the `expired` flag.
pub struct ApiKeyRow {
    pub id: i64,
    pub name: String,
    pub prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub expired: bool,
}

fn local_time(t: DateTime<Utc>) -> Markup {
    html! { span class="localtime" data-ts=(t.to_rfc3339()) { (crate::view::fmt_utc(&t)) } }
}

pub fn account(view: &AccountView) -> Markup {
    let body = html! {
        div class="phead" { div {
            h1 { "Account" }
            p class="crumb tight" { "Manage the browser sessions signed in as you and the API keys used for programmatic access." }
        } }
        @if let Some(token) = &view.new_token {
            div class="card accent" { div class="cb" {
                p class="flash ok" data-testid="api-key-created" { "Key created — copy it now. For security it won't be shown again." }
                div class="urlrow" { code data-testid="api-key-token" { (token) } button class="copy" type="button" data-copy=(token) { "Copy" } }
            } }
        }
        @if let Some(flash) = &view.password_flash {
            div class="card" { div class="cb" { p class="flash ok" data-testid="password-changed-flash" { (flash) } } }
        }
        @if view.has_password { (password_card(view)) }
        (sessions_card(view))
        @if view.other_count > 0 {
            div class="card" { div class="cb" {
                form method="post" action="/account/sessions/revoke-others" data-confirm="Revoke every other session? Only this device will remain signed in." {
                    (csrf_field(&view.csrf))
                    button class="btn danger" type="submit" data-testid="session-revoke-others" { "Revoke all other sessions" }
                }
            } }
        }
        (keys_card(view))
        (create_key_card(view))
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

fn password_card(view: &AccountView) -> Markup {
    html! {
        div class="card" {
            div class="ch" { h2 { "Change password" } }
            div class="cb" {
                @if let Some(error) = &view.password_error { p class="flash err" data-testid="password-error" { (error) } }
                p class="crumb tight" { "Changing it signs out every other session listed below. API keys keep working — revoke those separately." }
                form method="post" action="/account/password" {
                    (csrf_field(&view.csrf))
                    div class="field" {
                        label for="current_password" { "Current password" }
                        input id="current_password" name="current_password" type="password" autocomplete="current-password" data-testid="current-password-input" required;
                    }
                    div class="two" {
                        div class="field" {
                            label for="new_password" { "New password" }
                            input id="new_password" name="new_password" type="password" autocomplete="new-password" data-testid="new-password-input" required;
                        }
                        div class="field" {
                            label for="confirm_password" { "Confirm new password" }
                            input id="confirm_password" name="confirm_password" type="password" autocomplete="new-password" data-testid="confirm-password-input" required;
                        }
                    }
                    div class="formactions" { button class="btn primary" type="submit" data-testid="password-submit" { "Change password" } }
                }
            }
        }
    }
}

fn sessions_card(view: &AccountView) -> Markup {
    html! {
        div class="card" {
            div class="ch" { h2 { "Your sessions" } }
            @if view.sessions.is_empty() {
                p class="empty" data-testid="sessions-empty" { "No active sessions." }
            } @else {
                div class="cb" { div class="tscroll" { table {
                    thead { tr { th { "Started" } th { "Last seen" } th { "IP" } th { "Browser" } th { "Idle expiry" } th {} } }
                    tbody {
                        @for s in &view.sessions { (session_row(s, &view.csrf)) }
                    }
                } } }
            }
        }
    }
}

fn session_row(s: &SessionRow, csrf: &str) -> Markup {
    html! {
        tr data-testid={"session-row-" (s.handle)} {
            td class="mono" { @if let Some(t) = s.created_at { (local_time(t)) } @else { "—" } }
            td class="mono" { @if let Some(t) = s.last_seen_at { (local_time(t)) } @else { "never" } }
            td class="mono" { @if let Some(ip) = &s.ip { (ip) } @else { "—" } }
            td {
                @if let Some(ua) = &s.user_agent { (ua) } @else { "—" }
                @if s.current { " " span class="pill ok" data-testid="session-current" { "this device" } }
                @if s.sso { " " span class="pill log" data-testid="session-sso" { "SSO" } }
            }
            td class="mono" { (local_time(s.expires_at)) }
            td {
                form class="inline" method="post" action={"/account/sessions/" (s.handle) "/revoke"} data-confirm="Revoke this session? If it is your current session you will be signed out." {
                    (csrf_field(csrf))
                    button class="btn danger" type="submit" data-testid="session-revoke" { "revoke" }
                }
            }
        }
    }
}

fn keys_card(view: &AccountView) -> Markup {
    html! {
        div class="card" {
            div class="ch" { h2 { "Your keys" } }
            div class="cb" {
                p class="crumb tight" { "Authenticate programmatic requests with " code { "Authorization: Bearer <token>" } ". " a href="/api/docs" data-testid="api-docs-link" { "API reference" } " · " a href="/api/openapi.json" data-testid="api-openapi-link" { "OpenAPI spec" } "." }
            }
            @if view.keys.is_empty() {
                p class="empty" data-testid="api-keys-empty" { "No API keys yet — create one below." }
            } @else {
                div class="cb" { div class="tscroll" { table {
                    thead { tr { th { "Name" } th { "Prefix" } th { "Created" } th { "Last used" } th { "Expires" } th {} } }
                    tbody {
                        @for k in &view.keys { (key_row(k, &view.csrf)) }
                    }
                } } }
            }
        }
    }
}

fn key_row(k: &ApiKeyRow, csrf: &str) -> Markup {
    html! {
        tr data-testid={"api-key-row-" (k.id)} {
            td { (k.name) }
            td class="mono" { (k.prefix) "…" }
            td class="mono" { (local_time(k.created_at)) }
            td class="mono" { @if let Some(t) = k.last_used_at { (local_time(t)) } @else { "never" } }
            td class="mono" {
                @if let Some(t) = k.expires_at {
                    (local_time(t))
                    @if k.expired { " " span class="pill fail" data-testid={"api-key-expired-" (k.id)} { "expired" } }
                } @else { "never" }
            }
            td {
                form class="inline" method="post" action={"/account/api-keys/" (k.id) "/delete"} data-confirm="Revoke this API key?" {
                    (csrf_field(csrf))
                    button class="btn danger" type="submit" data-testid="api-key-delete" { "revoke" }
                }
            }
        }
    }
}

fn create_key_card(view: &AccountView) -> Markup {
    html! {
        div class="card flush" {
            div class="ch" { h2 { "Create API key" } }
            div class="cb" {
                @if let Some(error) = &view.key_error { p class="flash err" data-testid="api-key-error" { (error) } }
                form method="post" action="/account/api-keys" {
                    (csrf_field(&view.csrf))
                    div class="two" {
                        div class="field" {
                            label for="name" { "Name" }
                            input id="name" name="name" data-testid="api-key-name-input" placeholder="e.g. CI deploy" required;
                        }
                        div class="field" {
                            label for="expires_in" { "Expires in (optional)" }
                            // Blank means never, so the field is not `required`.
                            input id="expires_in" name="expires_in" list="expiry-list" autocomplete="off" data-testid="api-key-expires-input" placeholder="30d — blank = never";
                            datalist id="expiry-list" {
                                @for e in crate::view::expiries() { option value=(e) {} }
                            }
                        }
                    }
                    @if view.has_password {
                        div class="field" {
                            label for="key_current_password" { "Your password" }
                            input id="key_current_password" name="current_password" type="password" autocomplete="current-password" data-testid="api-key-password-input" required;
                            p class="crumb tight" { "A key keeps working after you sign out or change your password, so it is asked for again here." }
                        }
                    }
                    div class="formactions" { button class="btn primary" type="submit" data-testid="api-key-submit" { "Create key" } }
                }
            }
        }
    }
}
