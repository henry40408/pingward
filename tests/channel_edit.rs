//! Web-surface tests for editing a notification channel.
//!
//! The merge rules (`web::validate_channel_update`: a blank field keeps its
//! stored value, `kind` is immutable, `ntfy_token_clear` is the one escape hatch)
//! are covered over the API in `tests/api_v1_write.rs::patch_channel_*`, since
//! both surfaces call the same validator. Here: the edit page must never render
//! a stored delivery secret — `channels.config_json` holds webhook URLs and bot
//! tokens in plaintext.

use axum_test::TestServer;
use chrono::Utc;
use pingward::{app, models::ChannelKind, state::AppState, store::Store};
use serde_json::{Value, json};

mod common;

async fn server_as(username: &str, is_admin: bool) -> (TestServer, Store, i64) {
    let pool = pingward::db::connect("sqlite::memory:").await.unwrap();
    pingward::db::migrate(&pool, "sqlite::memory:")
        .await
        .unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user(username, Some(&phc), is_admin, Utc::now())
        .await
        .unwrap();
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", username),
            ("password", "pw"),
        ])
        .await
        .assert_status_see_other();
    (server, store, uid)
}

async fn csrf(store: &Store) -> String {
    common::newest_session_csrf(&store.pool).await
}

async fn project_of(store: &Store, uid: i64) -> i64 {
    store
        .create_project(uid, "p", "", None, None, Utc::now())
        .await
        .unwrap()
}

async fn channel(store: &Store, pid: i64, kind: ChannelKind, config: &Value) -> i64 {
    store
        .create_channel(pid, kind, "chan", &config.to_string(), Utc::now())
        .await
        .unwrap()
}

async fn stored_config(store: &Store, chid: i64) -> Value {
    let ch = store.find_channel(chid).await.unwrap().unwrap();
    serde_json::from_str(&ch.config_json).unwrap()
}

/// The core invariant, for every kind that stores a secret: the edit page renders
/// a `configured` pill instead of the value.
///
/// Asserted both ways — a page that 500'd, or rendered the wrong channel, would
/// trivially not contain the secret, so each case also asserts this really is the
/// edit form for that channel.
#[tokio::test]
async fn edit_form_never_renders_a_stored_secret() {
    let (server, store, uid) = server_as("alice", false).await;
    let pid = project_of(&store, uid).await;

    // (kind, stored config, the substrings that must NOT appear)
    let cases: Vec<(ChannelKind, Value, Vec<&str>)> = vec![
        (
            ChannelKind::Webhook,
            json!({ "url": "https://hooks.example.com/WEBHOOK-SECRET" }),
            vec!["WEBHOOK-SECRET"],
        ),
        (
            ChannelKind::Slack,
            json!({ "url": "https://hooks.slack.com/services/SLACK-SECRET" }),
            vec!["SLACK-SECRET"],
        ),
        (
            ChannelKind::Telegram,
            json!({ "token": "TG-BOT-SECRET", "chat_id": "4242" }),
            vec!["TG-BOT-SECRET"],
        ),
        (
            ChannelKind::Ntfy,
            json!({ "base_url": "https://ntfy.example.com", "topic": "alerts", "token": "NTFY-SECRET" }),
            vec!["NTFY-SECRET"],
        ),
        (
            ChannelKind::Pushover,
            json!({ "token": "PO-APP-SECRET", "user": "PO-USER-SECRET" }),
            vec!["PO-APP-SECRET", "PO-USER-SECRET"],
        ),
    ];

    for (kind, config, secrets) in cases {
        let chid = channel(&store, pid, kind, &config).await;
        let res = server.get(&format!("/channels/{chid}/edit")).await;
        res.assert_status_ok();
        let body = res.text();

        for secret in &secrets {
            assert!(
                !body.contains(secret),
                "{} edit form leaked a stored secret ({secret})",
                kind.as_str()
            );
        }
        // Positive controls: this really is the edit form for this channel.
        assert!(
            body.contains("value=\"chan\""),
            "{} edit form must pre-fill the channel name",
            kind.as_str()
        );
        assert!(
            body.contains(&format!(">{}<", kind.as_str())),
            "{} edit form must render the immutable kind",
            kind.as_str()
        );
        assert!(
            body.contains(">configured<"),
            "{} edit form must show a configured pill for the secret it hid",
            kind.as_str()
        );
        assert!(
            body.contains("placeholder=\"unchanged\""),
            "{} edit form must mark the blank secret input as unchanged",
            kind.as_str()
        );
    }
}

/// Identifiers are safe to pre-fill, and must be, or an edit would silently need
/// them re-typed.
#[tokio::test]
async fn edit_form_prefills_non_secret_fields() {
    let (server, store, uid) = server_as("alice", false).await;
    let pid = project_of(&store, uid).await;

    let tg = channel(
        &store,
        pid,
        ChannelKind::Telegram,
        &json!({ "token": "TG-SECRET", "chat_id": "4242" }),
    )
    .await;
    let body = server.get(&format!("/channels/{tg}/edit")).await.text();
    assert!(body.contains("value=\"4242\""), "chat id must pre-fill");

    let ntfy = channel(
        &store,
        pid,
        ChannelKind::Ntfy,
        &json!({ "base_url": "https://ntfy.example.com", "topic": "alerts", "token": "" }),
    )
    .await;
    let body = server.get(&format!("/channels/{ntfy}/edit")).await.text();
    assert!(body.contains("value=\"https://ntfy.example.com\""));
    assert!(body.contains("value=\"alerts\""));
    assert!(
        body.contains(">not set<"),
        "an unset optional secret must read as not set, not as configured"
    );
}

/// The form's secret input is blank, so the browser cannot send the current
/// value back: the merge is what keeps it.
#[tokio::test]
async fn edit_renames_without_resubmitting_the_secret() {
    let (server, store, uid) = server_as("alice", false).await;
    let pid = project_of(&store, uid).await;
    let secret = "https://hooks.example.com/KEEP-ME";
    let chid = channel(&store, pid, ChannelKind::Webhook, &json!({ "url": secret })).await;
    let token = csrf(&store).await;

    server
        .post(&format!("/channels/{chid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "renamed"),
            ("webhook_url", ""),
        ])
        .await
        .assert_status_see_other();

    let ch = store.find_channel(chid).await.unwrap().unwrap();
    assert_eq!(ch.name, "renamed");
    assert_eq!(stored_config(&store, chid).await["url"], secret);
}

/// A submitted secret overwrites; the page still shows neither old nor new value.
#[tokio::test]
async fn edit_rotates_a_secret_without_rendering_it() {
    let (server, store, uid) = server_as("alice", false).await;
    let pid = project_of(&store, uid).await;
    let chid = channel(
        &store,
        pid,
        ChannelKind::Webhook,
        &json!({ "url": "https://hooks.example.com/OLD" }),
    )
    .await;
    let token = csrf(&store).await;

    server
        .post(&format!("/channels/{chid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "chan"),
            ("webhook_url", "https://hooks.example.com/NEW-SECRET"),
        ])
        .await
        .assert_status_see_other();

    assert_eq!(
        stored_config(&store, chid).await["url"],
        "https://hooks.example.com/NEW-SECRET"
    );
    let body = server.get(&format!("/channels/{chid}/edit")).await.text();
    assert!(
        !body.contains("NEW-SECRET"),
        "the rotated secret must not render"
    );
    assert!(!body.contains("/OLD"));
}

/// The checkbox is the only way to remove an optional secret, so its `value` must
/// parse as a bool — anything else silently no-ops.
#[tokio::test]
async fn edit_clears_the_ntfy_token_via_the_checkbox() {
    let (server, store, uid) = server_as("alice", false).await;
    let pid = project_of(&store, uid).await;
    let chid = channel(
        &store,
        pid,
        ChannelKind::Ntfy,
        &json!({ "base_url": "https://ntfy.sh", "topic": "alerts", "token": "NTFY-SECRET" }),
    )
    .await;
    let token = csrf(&store).await;

    // Rendered only when a token is stored, with a value the handler's bool parses.
    let body = server.get(&format!("/channels/{chid}/edit")).await.text();
    assert!(
        body.contains("name=\"ntfy_token_clear\" value=\"true\""),
        "the clear checkbox must submit a parseable bool"
    );

    server
        .post(&format!("/channels/{chid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "chan"),
            ("ntfy_base_url", "https://ntfy.sh"),
            ("ntfy_topic", "alerts"),
            ("ntfy_token", ""),
            ("ntfy_token_clear", "true"),
        ])
        .await
        .assert_status_see_other();

    let cfg = stored_config(&store, chid).await;
    assert_eq!(cfg["token"], "");
    assert_eq!(cfg["topic"], "alerts");
    // With the token gone the checkbox is not rendered: it would be a no-op control.
    let body = server.get(&format!("/channels/{chid}/edit")).await.text();
    assert!(!body.contains("name=\"ntfy_token_clear\""));
}

/// A rejected edit re-renders with the error, still without printing the stored
/// secret.
#[tokio::test]
async fn rejected_edit_re_renders_without_leaking_or_saving() {
    let (server, store, uid) = server_as("alice", false).await;
    let pid = project_of(&store, uid).await;
    let secret = "https://hooks.example.com/KEEP-ME";
    let chid = channel(&store, pid, ChannelKind::Webhook, &json!({ "url": secret })).await;
    let token = csrf(&store).await;

    // A blank name on an edit is legal (it keeps the stored one), so the
    // rejection has to come from a kind-specific field: a channel whose stored
    // config has no URL, submitted blank.
    let empty = channel(&store, pid, ChannelKind::Webhook, &json!({})).await;
    let res = server
        .post(&format!("/channels/{empty}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "renamed"),
            ("webhook_url", ""),
        ])
        .await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("a webhook URL is required"));
    assert_eq!(
        store.find_channel(empty).await.unwrap().unwrap().name,
        "chan",
        "a rejected edit must not have applied the name"
    );

    // The other channel's secret is untouched.
    let res = server
        .post(&format!("/channels/{chid}"))
        .form(&[("_csrf", token.as_str()), ("name", ""), ("webhook_url", "")])
        .await;
    res.assert_status_see_other();
    assert_eq!(
        store.find_channel(chid).await.unwrap().unwrap().name,
        "chan",
        "a blank name keeps the stored one"
    );
    assert_eq!(stored_config(&store, chid).await["url"], secret);
}

/// `kind` is immutable: the edit form offers no select, and a submitted kind is
/// ignored rather than reinterpreting the stored config.
#[tokio::test]
async fn edit_form_does_not_offer_a_kind_select() {
    let (server, store, uid) = server_as("alice", false).await;
    let pid = project_of(&store, uid).await;
    let chid = channel(
        &store,
        pid,
        ChannelKind::Webhook,
        &json!({ "url": "https://hooks.example.com/S" }),
    )
    .await;

    let create = server.get(&format!("/projects/{pid}/channels/new")).await;
    assert!(
        create.text().contains("<select name=\"kind\""),
        "the create form must still offer the kind select"
    );

    let edit = server.get(&format!("/channels/{chid}/edit")).await;
    assert!(
        !edit.text().contains("<select name=\"kind\""),
        "the edit form must render the kind as static text, not a select"
    );

    let token = csrf(&store).await;
    server
        .post(&format!("/channels/{chid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "chan"),
            ("kind", "slack"),
            ("slack_url", "https://hooks.slack.com/services/X"),
        ])
        .await
        .assert_status_see_other();
    assert_eq!(
        store.find_channel(chid).await.unwrap().unwrap().kind,
        ChannelKind::Webhook,
        "a submitted kind must be ignored on edit"
    );
}

/// The project page has to expose the new surface, or it is unreachable.
#[tokio::test]
async fn project_page_links_to_the_channel_edit_form() {
    let (server, store, uid) = server_as("alice", false).await;
    let pid = project_of(&store, uid).await;
    let chid = channel(
        &store,
        pid,
        ChannelKind::Webhook,
        &json!({ "url": "https://hooks.example.com/S" }),
    )
    .await;

    let body = server.get(&format!("/projects/{pid}")).await.text();
    assert!(
        body.contains(&format!("/channels/{chid}/edit")),
        "the project page must link to each channel's edit form"
    );
}

/// The admin surface reuses the template and core, so the non-leakage guarantee
/// carries over — and its form must post back to `/admin/...`, or saving 404s
/// (the bug PR #77 found in `admin_project_delete`).
#[tokio::test]
async fn admin_can_edit_another_users_channel_without_leaking() {
    let (server, store, _admin_uid) = server_as("root", true).await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let other = store
        .create_user("bob", Some(&phc), false, Utc::now())
        .await
        .unwrap();
    let pid = project_of(&store, other).await;
    let secret = "https://hooks.example.com/BOB-SECRET";
    let chid = channel(&store, pid, ChannelKind::Webhook, &json!({ "url": secret })).await;

    let res = server.get(&format!("/admin/channels/{chid}/edit")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        !body.contains("BOB-SECRET"),
        "admin edit form leaked a secret"
    );
    assert!(
        body.contains(&format!("action=\"/admin/channels/{chid}\"")),
        "the admin form must post back to the admin route"
    );

    let token = csrf(&store).await;
    server
        .post(&format!("/admin/channels/{chid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "renamed-by-admin"),
            ("webhook_url", ""),
        ])
        .await
        .assert_status_see_other();
    let ch = store.find_channel(chid).await.unwrap().unwrap();
    assert_eq!(ch.name, "renamed-by-admin");
    assert_eq!(stored_config(&store, chid).await["url"], secret);
}
