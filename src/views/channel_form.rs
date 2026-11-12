use maud::{Markup, html};

use super::layout::{Chrome, csrf_field, page};

pub struct ChannelFormView {
    pub csrf: String,
    pub is_admin: bool,
    pub admin: bool,
    pub project_id: i64,
    pub error: Option<String>,
    pub smtp_available: bool,
    /// `Some` when editing (heading, action, fixed kind, config block).
    pub edit: Option<ChannelEditView>,
}

/// The edit page's only view of a stored config, making non-leakage a type property
/// (`ChannelDto` does the same for the API). Webhook/Slack URLs are capability secrets;
/// chat ids, ntfy server/topic and email recipients are safe to pre-fill.
pub struct ChannelEditView {
    pub id: i64,
    pub kind: &'static str,
    pub name: String,
    // -- pre-filled, non-secret --
    pub telegram_chat_id: String,
    pub ntfy_base_url: String,
    pub ntfy_topic: String,
    pub email_to: String,
    // -- configured/not-set pills only. Shared `url`/`token` keys can't collide:
    //    only `kind`'s block renders.
    pub has_webhook_url: bool,
    pub has_slack_url: bool,
    pub has_telegram_token: bool,
    pub has_ntfy_token: bool,
    pub has_pushover_token: bool,
    pub has_pushover_user: bool,
}

/// A secret is never re-rendered — only whether one is stored.
fn secret_status(is_set: bool) -> Markup {
    html! {
        div class="help" {
            @if is_set { span class="pill ok" { "configured" } } @else { span class="note" { "not set" } }
            " — leave blank to keep the stored value."
        }
    }
}

pub fn channel_form(view: &ChannelFormView) -> Markup {
    let base = if view.admin { "/admin" } else { "" };
    let edit = view.edit.as_ref();
    // Empty when creating: every block renders and app.css's `:has()` rules show the
    // selected kind's. When editing, only the immutable kind's.
    let ek = edit.map_or("", |e| e.kind);
    let action = match edit {
        Some(e) => format!("{base}/channels/{}", e.id),
        None => format!("{base}/projects/{}/channels", view.project_id),
    };
    let body = html! {
        h1 { @if edit.is_some() { "Edit channel" } @else { "New channel" } }
        @if let Some(error) = &view.error { p class="flash err" { (error) } }
        div class="card" { div class="cb" {
            form method="post" action=(action) {
                (csrf_field(&view.csrf))
                div class="field" {
                    label for="name" { "Name" }
                    input id="name" name="name" required value=[edit.map(|e| e.name.as_str())];
                }
                div class="field" {
                    label for="kind" { "Kind" }
                    @if let Some(e) = edit {
                        p class="mono" data-testid="channel-kind-static" { (e.kind) }
                        div class="help" { "A channel's kind can't be changed — delete it and create a new one instead." }
                    } @else {
                        select name="kind" id="kind" {
                            option value="webhook" { "webhook" }
                            option value="slack" { "slack" }
                            option value="telegram" { "telegram" }
                            option value="ntfy" { "ntfy" }
                            option value="pushover" { "pushover" }
                            @if view.smtp_available { option value="email" { "email" } }
                        }
                    }
                }
                @if ek.is_empty() || ek == "webhook" { (webhook_block(edit)) }
                @if ek.is_empty() || ek == "slack" { (slack_block(edit)) }
                @if ek.is_empty() || ek == "telegram" { (telegram_block(edit)) }
                @if ek.is_empty() || ek == "ntfy" { (ntfy_block(edit)) }
                @if ek.is_empty() || ek == "pushover" { (pushover_block(edit)) }
                // Editable even with SMTP off; only creation is gated on it.
                @if ek == "email" || (ek.is_empty() && view.smtp_available) { (email_block(edit)) }
                div class="btnrow" {
                    button class="btn primary" type="submit" { @if edit.is_some() { "Save changes" } @else { "Create channel" } }
                    @if edit.is_some() { a class="btn" href={(base) "/projects/" (view.project_id)} { "Cancel" } }
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

fn unchanged_if(editing: bool) -> Option<&'static str> {
    editing.then_some("unchanged")
}

fn webhook_block(edit: Option<&ChannelEditView>) -> Markup {
    html! {
        div class="cfg" data-kind="webhook" { div class="field" {
            label for="webhook_url" { "Webhook URL" }
            input id="webhook_url" name="webhook_url" class=[edit.map(|_| "mono")] placeholder=[unchanged_if(edit.is_some())];
            @if let Some(e) = edit { (secret_status(e.has_webhook_url)) }
        } }
    }
}

fn slack_block(edit: Option<&ChannelEditView>) -> Markup {
    let placeholder = if edit.is_some() {
        "unchanged"
    } else {
        "https://hooks.slack.com/services/..."
    };
    html! {
        div class="cfg" data-kind="slack" { div class="field" {
            label for="slack_url" { "Slack incoming webhook URL" }
            input id="slack_url" name="slack_url" class="mono" placeholder=(placeholder);
            @if let Some(e) = edit {
                (secret_status(e.has_slack_url))
            } @else {
                div class="help" { "Paste the incoming webhook URL generated by your Slack app." }
            }
        } }
    }
}

fn telegram_block(edit: Option<&ChannelEditView>) -> Markup {
    html! {
        div class="cfg" data-kind="telegram" {
            div class="field" {
                label for="telegram_token" { "Bot token" }
                input id="telegram_token" name="telegram_token" placeholder=[unchanged_if(edit.is_some())];
                @if let Some(e) = edit { (secret_status(e.has_telegram_token)) }
            }
            div class="field" {
                label for="telegram_chat_id" { "Chat id" }
                input id="telegram_chat_id" name="telegram_chat_id" value=[edit.map(|e| e.telegram_chat_id.as_str())];
            }
        }
    }
}

fn ntfy_block(edit: Option<&ChannelEditView>) -> Markup {
    html! {
        div class="cfg" data-kind="ntfy" {
            div class="field" {
                label for="ntfy_base_url" { "Server base URL" }
                input id="ntfy_base_url" name="ntfy_base_url" placeholder="https://ntfy.sh" value=[edit.map(|e| e.ntfy_base_url.as_str())];
            }
            div class="field" {
                label for="ntfy_topic" { "Topic" }
                input id="ntfy_topic" name="ntfy_topic" value=[edit.map(|e| e.ntfy_topic.as_str())];
            }
            div class="field" {
                label for="ntfy_token" { "Token (optional)" }
                input id="ntfy_token" name="ntfy_token" placeholder=[unchanged_if(edit.is_some())];
                @if let Some(e) = edit {
                    (secret_status(e.has_ntfy_token))
                    // Blank keeps the stored token, so removing it needs this.
                    @if e.has_ntfy_token {
                        label class="chk" { input type="checkbox" name="ntfy_token_clear" value="true" data-testid="ntfy-token-clear"; span class="nm" { "Remove the stored token" } }
                    }
                }
            }
        }
    }
}

fn pushover_block(edit: Option<&ChannelEditView>) -> Markup {
    html! {
        div class="cfg" data-kind="pushover" {
            div class="field" {
                label for="pushover_token" { "Application token" }
                input id="pushover_token" name="pushover_token" placeholder=[unchanged_if(edit.is_some())];
                @if let Some(e) = edit { (secret_status(e.has_pushover_token)) }
            }
            div class="field" {
                label for="pushover_user" { "User/group key" }
                input id="pushover_user" name="pushover_user" placeholder=[unchanged_if(edit.is_some())];
                @if let Some(e) = edit { (secret_status(e.has_pushover_user)) }
            }
        }
    }
}

fn email_block(edit: Option<&ChannelEditView>) -> Markup {
    html! {
        div class="cfg" data-kind="email" { div class="field" {
            label for="email_to" { "Recipient email" }
            input id="email_to" name="email_to" type="email" value=[edit.map(|e| e.email_to.as_str())];
        } }
    }
}
