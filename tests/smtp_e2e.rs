//! Real SMTP delivery test against a live relay (Mailpit).
//!
//! Skipped unless `PINGWARD_TEST_SMTP_HOST` is set (CI points it at the Mailpit
//! service container). It is the one path unit tests cannot cover: a message
//! crossing a real SMTP connection, asserted back via Mailpit's REST API.

use chrono::Utc;
use pingward::config::{SmtpConfig, SmtpTls};
use pingward::models::{Channel, ChannelKind};
use pingward::notify::{EventDetail, EventKind, NotificationEvent, notifier_for};

#[tokio::test]
async fn email_channel_delivers_over_smtp_to_relay() {
    let Ok(host) = std::env::var("PINGWARD_TEST_SMTP_HOST") else {
        eprintln!(
            "PINGWARD_TEST_SMTP_HOST unset — skipping email_channel_delivers_over_smtp_to_relay"
        );
        return;
    };
    let port: u16 = std::env::var("PINGWARD_TEST_SMTP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1025);
    let api = std::env::var("PINGWARD_TEST_MAILPIT_API")
        .expect("PINGWARD_TEST_MAILPIT_API must be set when PINGWARD_TEST_SMTP_HOST is");

    // Start from an empty mailbox so the assertion is deterministic.
    reqwest::Client::new()
        .delete(format!("{api}/api/v1/messages"))
        .send()
        .await
        .expect("clear mailpit mailbox");

    // Plaintext relay (TLS=none), no auth — matches the Mailpit default.
    let smtp = SmtpConfig {
        host,
        port,
        username: None,
        password: None,
        from: "alerts@example.com".into(),
        tls: SmtpTls::None,
    };
    let channel = Channel {
        id: 1,
        project_id: 1,
        kind: ChannelKind::Email,
        name: "ops".into(),
        config_json: r#"{"to":"ops@example.com"}"#.into(),
        created_at: Utc::now(),
    };
    let ev = NotificationEvent {
        check_id: 1,
        check_name: "backup".into(),
        event: EventKind::Down,
        at: Utc::now(),
        project_id: 1,
        detail: EventDetail::default(),
    };

    // The real notifier path: notifier_for -> build_email -> lettre -> SMTP.
    let notifier = notifier_for(&channel, Some(&smtp)).expect("email notifier for configured SMTP");
    notifier.send(&ev).await.expect("email delivered to relay");

    let messages: serde_json::Value = reqwest::get(format!("{api}/api/v1/messages"))
        .await
        .expect("query mailpit")
        .json()
        .await
        .expect("parse mailpit response");

    assert_eq!(
        messages["total"], 1,
        "exactly one message should be delivered; got: {messages}"
    );
    let msg = &messages["messages"][0];
    assert_eq!(msg["From"]["Address"], "alerts@example.com", "got: {msg}");
    assert_eq!(msg["To"][0]["Address"], "ops@example.com", "got: {msg}");
    let subject = msg["Subject"].as_str().unwrap_or_default();
    assert!(
        subject.contains("pingward") && subject.contains("backup"),
        "subject was: {subject}"
    );
}
