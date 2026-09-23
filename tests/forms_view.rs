use axum_test::TestServer;
use pingward::{app, state::AppState, store::Store};

mod common;

async fn server() -> (TestServer, Store) {
    let pool = pingward::db::connect("sqlite::memory:").await.unwrap();
    pingward::db::migrate(&pool, "sqlite::memory:")
        .await
        .unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), common::test_config());
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    (server, store)
}

async fn logged_in_server() -> (TestServer, Store, i64) {
    let (mut server, store) = server().await;
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    let csrf = common::anonymous_csrf(&mut server).await;
    server
        .post("/login")
        .form(&[
            ("_csrf", csrf.as_str()),
            ("username", "admin"),
            ("password", "pw"),
        ])
        .await;
    (server, store, uid)
}

async fn server_with_project() -> (TestServer, Store, i64) {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    (server, store, pid)
}

async fn csrf_token(store: &Store) -> String {
    common::newest_session_csrf(&store.pool).await
}

#[tokio::test]
async fn channel_form_is_restyled_and_keeps_fields() {
    let (server, _store, pid) = server_with_project().await;
    let res = server.get(&format!("/projects/{pid}/channels/new")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"field\""), "form not restyled");
    assert!(body.contains("name=\"webhook_url\""), "webhook field lost");
}

/// Every field name `CheckForm` reads.
#[tokio::test]
async fn check_form_is_restyled_and_keeps_fields() {
    let (server, _store, pid) = server_with_project().await;
    let res = server.get(&format!("/projects/{pid}/checks/new")).await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"field\""), "form not restyled");
    for name in [
        "name",
        "description",
        "schedule_kind",
        "period_secs",
        "cron_expr",
        "grace_secs",
        "timezone",
        "scan_interval_secs",
        "max_runtime_secs",
        "nag_interval_secs",
    ] {
        assert!(
            body.contains(&format!("name=\"{name}\"")),
            "check form lost field {name}"
        );
    }
}

/// Every field name `ProjectForm` reads.
#[tokio::test]
async fn project_form_is_restyled_and_keeps_fields() {
    let (server, _store, _uid) = logged_in_server().await;
    let res = server.get("/projects/new").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"field\""), "form not restyled");
    for name in [
        "name",
        "description",
        "scan_interval_secs",
        "nag_interval_secs",
    ] {
        assert!(
            body.contains(&format!("name=\"{name}\"")),
            "project form lost field {name}"
        );
    }
}

#[tokio::test]
async fn project_description_round_trips_and_is_length_validated() {
    let (server, store, uid) = logged_in_server().await;
    let token = csrf_token(&store).await;
    let res = server
        .post("/projects")
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "proj"),
            ("description", "**bold** desc"),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_see_other();
    let projects = store.list_projects_for_user(uid).await.unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0].description, "**bold** desc");

    let pid = projects[0].id;
    let edit = server.get(&format!("/projects/{pid}/edit")).await;
    edit.assert_status_ok();
    assert!(
        edit.text().contains("**bold** desc"),
        "edit form must round-trip the stored description into the textarea"
    );

    // Both sides of `MAX_DESCRIPTION_CHARS` (2000).
    let too_long = "a".repeat(2001);
    let res = server
        .post(&format!("/projects/{pid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "proj"),
            ("description", too_long.as_str()),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_ok();
    assert!(
        res.text()
            .contains("description must be at most 2000 characters"),
        "2001-char description must be rejected with the exact spec'd message"
    );
    assert_eq!(
        store.find_project(pid).await.unwrap().unwrap().description,
        "**bold** desc",
        "the rejected update must not have overwritten the stored description"
    );

    let boundary = "b".repeat(2000);
    let res = server
        .post(&format!("/projects/{pid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "proj"),
            ("description", boundary.as_str()),
            ("scan_interval_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_see_other();
    assert_eq!(
        store.find_project(pid).await.unwrap().unwrap().description,
        boundary,
        "a 2000-char description is exactly at the limit and must be accepted"
    );
}

#[tokio::test]
async fn check_description_round_trips_and_is_length_validated() {
    let (server, store, pid) = server_with_project().await;
    let token = csrf_token(&store).await;
    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "backup"),
            ("description", "runs *nightly*"),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("cron_expr", ""),
            ("grace_secs", "300"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_see_other();
    let checks = store.list_checks_for_project(pid).await.unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].description, "runs *nightly*");

    let cid = checks[0].id;
    let edit = server.get(&format!("/checks/{cid}/edit")).await;
    edit.assert_status_ok();
    assert!(
        edit.text().contains("runs *nightly*"),
        "edit form must round-trip the stored description into the textarea"
    );

    let too_long = "a".repeat(2001);
    let res = server
        .post(&format!("/checks/{cid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "backup"),
            ("description", too_long.as_str()),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("cron_expr", ""),
            ("grace_secs", "300"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_ok();
    assert!(
        res.text()
            .contains("description must be at most 2000 characters"),
        "2001-char description must be rejected with the exact spec'd message"
    );

    let boundary = "b".repeat(2000);
    let res = server
        .post(&format!("/checks/{cid}"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "backup"),
            ("description", boundary.as_str()),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("cron_expr", ""),
            ("grace_secs", "300"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_see_other();
    assert_eq!(
        store.find_check(cid).await.unwrap().unwrap().description,
        boundary,
        "a 2000-char description is exactly at the limit and must be accepted"
    );
}

/// Pins `check_create_core`'s call to `Store::bind_all_project_channels`.
#[tokio::test]
async fn check_created_via_web_form_is_bound_to_existing_channels() {
    let (server, store, pid) = server_with_project().await;
    let token = csrf_token(&store).await;

    let c1 = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook1",
            r#"{"url":"http://x"}"#,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let c2 = store
        .create_channel(
            pid,
            pingward::models::ChannelKind::Webhook,
            "hook2",
            r#"{"url":"http://y"}"#,
            chrono::Utc::now(),
        )
        .await
        .unwrap();

    let res = server
        .post(&format!("/projects/{pid}/checks"))
        .form(&[
            ("_csrf", token.as_str()),
            ("name", "backup"),
            ("description", ""),
            ("schedule_kind", "period"),
            ("period_secs", "3600"),
            ("cron_expr", ""),
            ("grace_secs", "300"),
            ("timezone", "UTC"),
            ("scan_interval_secs", ""),
            ("max_runtime_secs", ""),
            ("nag_interval_secs", ""),
        ])
        .await;
    res.assert_status_see_other();

    let checks = store.list_checks_for_project(pid).await.unwrap();
    assert_eq!(checks.len(), 1);
    let cid = checks[0].id;

    let mut bound = store.bound_channel_ids(cid).await.unwrap();
    bound.sort_unstable();
    let mut expected = vec![c1, c2];
    expected.sort_unstable();
    assert_eq!(
        bound, expected,
        "a check created in a project with existing channels must come out bound to all of them"
    );
}

/// `current-password` offers the saved credential, `new-password` a generated
/// one; swapping them fails silently in the password manager.
#[tokio::test]
async fn credential_fields_declare_their_autocomplete_role() {
    let (server, _store) = server().await;

    let setup = server.get("/setup").await.text();
    assert!(
        setup.contains(r#"name="username" autocomplete="username""#),
        "{setup}"
    );
    assert!(
        setup.contains(r#"type="password" autocomplete="new-password""#),
        "/setup sets a password, so it must not be tagged current-password: {setup}"
    );

    let (server, store, _uid) = logged_in_server().await;

    // A logged-out server on the same store; the signed-in jar would bounce to `/`.
    let mut anon = TestServer::new(app(AppState::new(store, common::test_config())));
    anon.save_cookies();
    let login = anon.get("/login").await.text();
    assert!(
        login.contains(r#"name="username" autocomplete="username""#),
        "{login}"
    );
    assert!(
        login.contains(r#"type="password" autocomplete="current-password""#),
        "/login submits an existing credential: {login}"
    );

    // `/admin` edits *other* accounts, so its username field opts out of autofill.
    let admin = server.get("/admin").await.text();
    assert!(
        admin.contains(r#"name="username" autocomplete="off""#),
        "{admin}"
    );
    assert_eq!(
        admin.matches(r#"autocomplete="new-password""#).count(),
        2,
        "both the reset field and the add-user field must be new-password: {admin}"
    );

    let account = server.get("/account").await.text();
    assert!(
        account.contains(r#"autocomplete="current-password""#),
        "{account}"
    );
    assert_eq!(account.matches(r#"autocomplete="new-password""#).count(), 2);
}

// --- duration suggestion lists ---------------------------------------------
//
// Duration fields are wired to a `<datalist>`, and every suggestion must be one
// the handler accepts — the user picks it from the browser's own dropdown.

fn input_tag<'a>(body: &'a str, id: &str) -> &'a str {
    body.split('<')
        .find(|tag| tag.starts_with("input") && tag.contains(&format!("id=\"{id}\"")))
        .unwrap_or_else(|| panic!("no <input id=\"{id}\"> on the page"))
}

/// Exactly one `<datalist id="{id}">`, with an `<option>` for each of `want`.
fn assert_list(body: &str, id: &str, want: &[&str]) {
    assert!(!want.is_empty(), "the suggestion list itself is empty");
    assert_eq!(
        body.matches(&format!("<datalist id=\"{id}\">")).count(),
        1,
        "expected exactly one <datalist id=\"{id}\"> — a duplicate id leaves the \
         second one as dead markup"
    );
    for value in want {
        assert!(
            body.contains(&format!("<option value=\"{value}\">")),
            "list {id} is missing the suggestion {value:?}"
        );
    }
}

fn assert_wired(body: &str, id: &str, fields: &[&str]) {
    for field in fields {
        let tag = input_tag(body, field);
        assert!(
            tag.contains(&format!("list=\"{id}\"")),
            "field {field} is not wired to the {id} suggestions: {tag}"
        );
    }
}

#[tokio::test]
async fn check_form_duration_fields_offer_the_shared_suggestions() {
    let (server, _store, pid) = server_with_project().await;
    let body = server
        .get(&format!("/projects/{pid}/checks/new"))
        .await
        .text();
    assert_list(&body, "dur-list", pingward::view::durations());
    assert_wired(
        &body,
        "dur-list",
        &[
            "period_secs",
            "grace_secs",
            "scan_interval_secs",
            "max_runtime_secs",
            "nag_interval_secs",
        ],
    );
}

#[tokio::test]
async fn project_form_duration_fields_offer_the_shared_suggestions() {
    let (server, _store, _uid) = logged_in_server().await;
    let body = server.get("/projects/new").await.text();
    assert_list(&body, "dur-list", pingward::view::durations());
    assert_wired(
        &body,
        "dur-list",
        &["scan_interval_secs", "nag_interval_secs"],
    );
}

/// The retention fields are `SettingKind::Days`, so `5m` would be rejected.
#[tokio::test]
async fn admin_settings_duration_fields_offer_the_shared_suggestions() {
    let (server, _store, _uid) = logged_in_server().await;
    let body = server.get("/admin").await.text();
    assert_list(&body, "dur-list", pingward::view::durations());
    assert_wired(&body, "dur-list", &["scan_interval", "nag_interval"]);
    for days_field in [
        "pings_retention_days",
        "notifications_retention_days",
        "audit_retention_days",
    ] {
        let tag = input_tag(&body, days_field);
        assert!(
            !tag.contains("list=\"dur-list\""),
            "{days_field} counts days, not durations, and must not offer duration \
             suggestions: {tag}"
        );
    }
}

#[tokio::test]
async fn api_key_expiry_offers_its_own_suggestions() {
    let (server, _store, _uid) = logged_in_server().await;
    let body = server.get("/account").await.text();
    assert_list(&body, "expiry-list", pingward::view::expiries());
    assert_wired(&body, "expiry-list", &["expires_in"]);
    assert!(
        !body.contains("<datalist id=\"dur-list\">"),
        "/account has no interval fields, so the shared list has no business there"
    );
}

/// Each `view::durations` entry must survive all five duration fields.
#[tokio::test]
async fn every_suggested_duration_is_accepted_by_the_check_form() {
    let (server, store, pid) = server_with_project().await;
    let token = csrf_token(&store).await;
    for suggestion in pingward::view::durations() {
        let res = server
            .post(&format!("/projects/{pid}/checks"))
            .form(&[
                ("_csrf", token.as_str()),
                ("name", suggestion),
                ("description", ""),
                ("schedule_kind", "period"),
                ("period_secs", suggestion),
                ("cron_expr", ""),
                ("grace_secs", suggestion),
                ("timezone", "UTC"),
                ("scan_interval_secs", suggestion),
                ("max_runtime_secs", suggestion),
                ("nag_interval_secs", suggestion),
            ])
            .await;
        assert_eq!(
            res.status_code(),
            303,
            "the check form rejected its own suggestion {suggestion:?}"
        );
        let want = pingward::duration::parse_duration(suggestion).unwrap();
        let checks = store.list_checks_for_project(pid).await.unwrap();
        let stored = checks
            .iter()
            .find(|c| c.name == *suggestion)
            .unwrap_or_else(|| panic!("no check stored for {suggestion:?}"));
        assert_eq!(
            stored.period_secs,
            Some(want),
            "{suggestion:?} did not round-trip into the stored period"
        );
        assert_eq!(stored.grace_secs, want);
        assert_eq!(stored.scan_interval_secs, Some(want));
        assert_eq!(stored.max_runtime_secs, Some(want));
        assert_eq!(stored.nag_interval_secs, Some(want));
    }
}
