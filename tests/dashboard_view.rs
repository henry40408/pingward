use axum_test::TestServer;
use pingward::{app, config::Config, db, state::AppState, store::Store};

async fn logged_in_server() -> (TestServer, Store, i64) {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool, "sqlite::memory:").await.unwrap();
    let store = Store::new(pool);
    let state = AppState::new(store.clone(), Config::from_map(|_| None));
    let mut server = TestServer::new(app(state));
    server.save_cookies();
    let phc = pingward::auth::hash_password("pw").unwrap();
    let uid = store
        .create_user("admin", Some(&phc), true, chrono::Utc::now())
        .await
        .unwrap();
    server
        .post("/login")
        .form(&[("username", "admin"), ("password", "pw")])
        .await;
    (server, store, uid)
}

/// Create a project and a single (never-pinged, "new") check inside it.
async fn server_with_project_and_check() -> (TestServer, Store, i64, i64) {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(uid, "web", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    let cid = store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            ping_uuid: "cu",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    (server, store, pid, cid)
}

#[tokio::test]
async fn dashboard_renders_tiles_and_badges() {
    let (server, _store, _pid, _cid) = server_with_project_and_check().await;

    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("class=\"tiles\""), "summary tiles missing");
    assert!(body.contains("class=\"badge"), "status badge missing");
}

#[tokio::test]
async fn dashboard_shows_project_group_and_check_row() {
    let (server, _store, pid, cid) = server_with_project_and_check().await;

    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(body.contains("web"), "project group name missing");
    assert!(body.contains("1 checks"), "group check count missing");
    assert!(
        body.contains(&format!("/projects/{pid}")),
        "manage link missing"
    );
    assert!(
        body.contains(&format!("/checks/{cid}")),
        "check row link missing"
    );
    assert!(body.contains("class=\"badge new\""), "new badge missing");
    assert!(
        body.contains("class=\"status-dot new\""),
        "new status dot missing"
    );
}

/// A running check has no tile of its own — it counts under Up — but keeps its
/// per-row running badge. This pins both halves of the up/running tile merge.
#[tokio::test]
async fn dashboard_counts_running_check_under_up_and_keeps_the_row_badge() {
    let (server, store, pid, cid) = server_with_project_and_check().await;
    // Give `cid` an in-flight start (no finish) so `display_status` resolves
    // it to Running.
    store
        .mark_ping(
            cid,
            pingward::models::CheckStatus::New,
            None,
            Some(chrono::Utc::now()),
            None,
        )
        .await
        .unwrap();
    // A second, untouched check is "new" — not up, not running — so it proves
    // the Up tile counts the running check specifically, not every check.
    store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "idle",
            ping_uuid: "idle-uuid",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();
    // The running check is folded into Up (count 1), and there is no Running
    // tile at all any more.
    assert_tile(&body, "Up", 1);
    assert!(
        !body.contains(">Running</div>"),
        "the Running tile must be gone (running now counts under Up): {body}"
    );
    // The per-row running indicator survives the tile merge.
    assert_eq!(
        body.matches("class=\"badge running\"").count(),
        1,
        "exactly one check row should render the running badge"
    );
}

/// Dashboard descriptions: a project's and a check's `markdown::truncate_plain`
/// output must actually reach the rendered page, inside the `gdesc`/`cdesc`
/// elements respectively, with markdown markers stripped (not raw) and the
/// check's long description genuinely truncated (not the full string).
#[tokio::test]
async fn dashboard_shows_truncated_descriptions_with_markdown_stripped() {
    let (server, store, uid) = logged_in_server().await;
    let pid = store
        .create_project(
            uid,
            "web",
            "**Web** services project",
            None,
            None,
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    let long_desc = "**Nightly** backups of the primary database run every day and verify \
        checksum integrity end to end, catching silent corruption early before it can spread \
        further into downstream systems and backups.";
    assert!(
        pingward::markdown::to_plain(long_desc).chars().count() > 120,
        "test fixture must actually be long enough to exercise truncation"
    );
    store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "backup",
            description: long_desc,
            ping_uuid: "cu-long-desc",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();
    // Negative control: a second check with an EMPTY description must not
    // render a `cdesc` element at all — proves the
    // `{% if !c.description.is_empty() %}` guard works, and that the
    // assertions below aren't matching something incidental.
    store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "idle",
            ping_uuid: "cu-empty-desc",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();

    // Project description: markers stripped, rendered inside `gdesc`.
    assert!(
        body.contains("class=\"gdesc\">Web services project</span>"),
        "project description missing/not stripped in gdesc: {body}"
    );
    assert!(
        !body.contains("**Web**"),
        "raw markdown markers leaked into gdesc, truncate_plain did not run: {body}"
    );

    // Check description: truncated (ellipsis present, tail of the original
    // string absent) and markers stripped, rendered inside `cdesc`.
    // 120 characters of content plus the ellipsis. Asserted against a literal
    // rather than a `truncate_plain` call, so a broken truncation cannot make
    // this test agree with itself.
    let expected = "Nightly backups of the primary database run every day and verify checksum integrity end to end, catching silent corrupti…";
    assert!(
        body.contains(&format!(
            "class=\"cdesc\" data-testid=\"check-description-summary\">{expected}</div>"
        )),
        "check description missing/not truncated-and-stripped in cdesc: {body}"
    );
    assert_eq!(expected.chars().count(), 121);
    assert!(
        body.contains('…'),
        "truncated check description must contain an ellipsis: {body}"
    );
    assert!(
        !body.contains("downstream systems and backups"),
        "the tail of the untruncated description leaked into the page: {body}"
    );
    assert!(
        !body.contains("**Nightly**"),
        "raw markdown markers leaked into cdesc, truncate_plain did not run: {body}"
    );

    // Negative control: exactly one check row (the one with a non-empty
    // description) should render a `cdesc` element.
    assert_eq!(
        body.matches("data-testid=\"check-description-summary\"")
            .count(),
        1,
        "the empty-description check must not render a cdesc element"
    );
}

/// A description whose distinctive term sits past the 120-character summary
/// cut-off, so a test can tell "matched the raw description" apart from
/// "matched what the row happens to display".
const LONG_DESC: &str = "Copies the primary database to cold storage every night and verifies \
    checksums end to end so silent corruption is caught before it spreads, then uploads the \
    result to the offsite glacier vault.";

/// Two projects with four checks between them, shaped so every dashboard filter
/// dimension (project name, project description, check name, check description)
/// can be exercised by a term unique to it.
async fn server_with_two_projects() -> TestServer {
    let (server, store, uid) = logged_in_server().await;
    let now = chrono::Utc::now();
    let web = store
        .create_project(uid, "web", "Public **frontend** services", None, None, now)
        .await
        .unwrap();
    let infra = store
        .create_project(uid, "infra", "Datacenter plumbing", None, None, now)
        .await
        .unwrap();
    for (project_id, name, description, uuid) in [
        (web, "backup", LONG_DESC, "cu-backup"),
        (web, "deploy", "", "cu-deploy"),
        (
            infra,
            "rotate-certs",
            "Renews TLS certificates",
            "cu-rotate",
        ),
        (infra, "vacuum", "", "cu-vacuum"),
    ] {
        store
            .create_check(&pingward::store::NewCheck {
                project_id,
                name,
                description,
                ping_uuid: uuid,
                kind: pingward::models::ScheduleKind::Period,
                period_secs: Some(3600),
                grace_secs: 300,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
    }
    server
}

/// Assert a summary tile shows exactly `n`. Matches the tile's own markup, so a
/// counter that stops following the filter fails here rather than passing on a
/// bare `body.contains("1")`.
fn assert_tile(body: &str, label: &str, n: usize) {
    let want = format!("<div class=\"n\">{n}</div><div class=\"l\">{label}</div>");
    assert!(
        body.contains(&want),
        "expected {label} tile to show {n}: {body}"
    );
}

#[tokio::test]
async fn dashboard_unfiltered_shows_every_project_and_no_clear_link() {
    let server = server_with_two_projects().await;
    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();

    assert!(body.contains(">web</h2>"), "web group missing: {body}");
    assert!(body.contains(">infra</h2>"), "infra group missing: {body}");
    assert_tile(&body, "Total", 4);
    assert!(
        !body.contains("dashboard-filter-clear"),
        "clear affordance must only appear while a filter is active: {body}"
    );
}

/// A blank or whitespace-only `q` is the unfiltered view, not a filter that
/// matches everything by accident — the "clear" link must stay hidden.
#[tokio::test]
async fn dashboard_blank_query_is_treated_as_unfiltered() {
    let server = server_with_two_projects().await;
    let body = server.get("/?q=%20%20").await.text();

    assert!(body.contains(">web</h2>"), "web group missing: {body}");
    assert!(body.contains(">infra</h2>"), "infra group missing: {body}");
    assert_tile(&body, "Total", 4);
    assert!(
        !body.contains("dashboard-filter-clear"),
        "whitespace-only q must not count as an active filter: {body}"
    );
}

#[tokio::test]
async fn dashboard_filter_by_check_name_drops_other_projects_and_narrows_counters() {
    let server = server_with_two_projects().await;
    let body = server.get("/?q=rotate").await.text();

    assert!(body.contains(">infra</h2>"), "infra group missing: {body}");
    assert!(
        body.contains(">rotate-certs</div>"),
        "match missing: {body}"
    );
    assert!(
        !body.contains(">web</h2>"),
        "a project with no matching check must not render: {body}"
    );
    assert!(
        !body.contains(">vacuum</div>"),
        "a sibling check that does not match must not render: {body}"
    );
    // The counters follow the filter: one visible check, and it is "new".
    assert_tile(&body, "Total", 1);
    assert_tile(&body, "Up", 0);
    assert!(
        body.contains("dashboard-filter-clear"),
        "an active filter must offer a way out: {body}"
    );
    assert!(
        body.contains("value=\"rotate\""),
        "the search box must echo the active term: {body}"
    );
}

/// A project-level hit shows the project whole. Filtering by a term that exists
/// only in the project's own description must still list checks that do not
/// match it themselves, rather than rendering a header above an empty list.
#[tokio::test]
async fn dashboard_filter_by_project_description_keeps_all_of_its_checks() {
    let server = server_with_two_projects().await;
    let body = server.get("/?q=plumbing").await.text();

    assert!(body.contains(">infra</h2>"), "infra group missing: {body}");
    assert!(
        body.contains(">rotate-certs</div>") && body.contains(">vacuum</div>"),
        "a project-level match must keep every check in that project: {body}"
    );
    assert!(
        !body.contains(">web</h2>"),
        "the non-matching project must not render: {body}"
    );
    assert_tile(&body, "Total", 2);
}

#[tokio::test]
async fn dashboard_filter_by_project_name_keeps_all_of_its_checks() {
    let server = server_with_two_projects().await;
    let body = server.get("/?q=infra").await.text();

    assert!(
        body.contains(">rotate-certs</div>") && body.contains(">vacuum</div>"),
        "a project-name match must keep every check in that project: {body}"
    );
    assert!(!body.contains(">web</h2>"), "web must not render: {body}");
    assert_tile(&body, "Total", 2);
}

/// Matching runs over the **raw** description, not the 120-character summary the
/// row displays. The term below appears only in the truncated-away tail, so this
/// fails if the filter is ever pointed at `CheckRow::description`.
#[tokio::test]
async fn dashboard_filter_matches_description_text_beyond_the_visible_summary() {
    let server = server_with_two_projects().await;
    let plain = pingward::markdown::to_plain(LONG_DESC);
    assert!(
        !pingward::markdown::truncate_plain(LONG_DESC, 120).contains("glacier"),
        "fixture must place the search term past the summary cut-off: {plain}"
    );

    let body = server.get("/?q=glacier").await.text();
    assert!(body.contains(">backup</div>"), "match missing: {body}");
    // "glacier" does appear once, echoed into the search box — but the tail it
    // came from must not be rendered, proving the match came from the stored
    // description rather than anything on screen.
    assert!(
        !body.contains("offsite glacier vault"),
        "the matched tail is past the summary cut-off, so it must not render: {body}"
    );
    assert_eq!(
        body.matches("glacier").count(),
        1,
        "the term should appear only in the echoed search box: {body}"
    );
    assert!(
        !body.contains(">deploy</div>"),
        "a sibling check that does not match must not render: {body}"
    );
    assert_tile(&body, "Total", 1);
}

#[tokio::test]
async fn dashboard_filter_is_case_insensitive() {
    let server = server_with_two_projects().await;
    // An uppercase query against lowercase stored data.
    let body = server.get("/?q=ROTATE-Certs").await.text();
    assert!(
        body.contains(">rotate-certs</div>"),
        "an uppercase term must match lowercase stored text: {body}"
    );
    assert_tile(&body, "Total", 1);

    // ...and the other direction, which is what folding the *haystack* buys:
    // "TLS" is stored uppercase, so a lowercase query must still find it.
    // Without this, dropping `to_lowercase()` on the haystack passes the suite.
    let body = server.get("/?q=tls").await.text();
    assert!(
        body.contains(">rotate-certs</div>"),
        "a lowercase term must match uppercase stored text: {body}"
    );
    assert_tile(&body, "Total", 1);
}

/// "Nothing matched" is a different state from "you have no projects" — they
/// must not collapse into the same message, or a user with a typo is told to
/// create a project they already have.
#[tokio::test]
async fn dashboard_no_results_state_is_distinct_from_the_empty_state() {
    let server = server_with_two_projects().await;
    let body = server.get("/?q=nonesuch").await.text();

    assert!(
        body.contains("dashboard-no-results"),
        "no-results state missing: {body}"
    );
    assert!(
        !body.contains("dashboard-empty"),
        "a user who owns projects must not be told they have none: {body}"
    );
    assert!(
        body.contains("Clear the filter"),
        "no-results state must offer a way back: {body}"
    );
    assert_tile(&body, "Total", 0);
}

/// One project whose four checks each resolve to a different display status:
/// `web` → Up, `job` → Running, `cron` → Late, `db` → Down. Lets a test point
/// the status filter at any bucket and assert both what shows and what the
/// tiles count.
async fn server_with_mixed_statuses() -> TestServer {
    use pingward::models::CheckStatus;
    let (server, store, uid) = logged_in_server().await;
    let now = chrono::Utc::now();
    let pid = store
        .create_project(uid, "services", "", None, None, now)
        .await
        .unwrap();
    async fn mk(store: &Store, pid: i64, name: &str, uuid: &str) -> i64 {
        store
            .create_check(&pingward::store::NewCheck {
                project_id: pid,
                name,
                ping_uuid: uuid,
                kind: pingward::models::ScheduleKind::Period,
                period_secs: Some(3600),
                grace_secs: 300,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap()
    }
    // Up: pinged, next run comfortably in the future (not late), not in flight.
    let web = mk(&store, pid, "web", "cu-web").await;
    store
        .mark_ping(
            web,
            CheckStatus::Up,
            Some(now),
            None,
            Some(now + chrono::Duration::seconds(3600)),
        )
        .await
        .unwrap();
    // Running: an in-flight start with no finish (last_start > last_ping).
    let job = mk(&store, pid, "job", "cu-job").await;
    store
        .mark_ping(job, CheckStatus::New, None, Some(now), None)
        .await
        .unwrap();
    // Late: stored Up, but `now` sits inside (expected, due] — due 100s out,
    // 300s grace, so expected was 200s ago.
    let cron = mk(&store, pid, "cron", "cu-cron").await;
    store
        .mark_ping(
            cron,
            CheckStatus::Up,
            Some(now - chrono::Duration::seconds(3500)),
            None,
            Some(now + chrono::Duration::seconds(100)),
        )
        .await
        .unwrap();
    // Down.
    let db = mk(&store, pid, "db", "cu-db").await;
    store
        .mark_ping(db, CheckStatus::Down, Some(now), None, None)
        .await
        .unwrap();
    server
}

/// Whether a check row for `name` is rendered (matches the row's `nm` cell, so
/// it can't be fooled by the name appearing in the search box or a project
/// header).
fn shows_check(body: &str, name: &str) -> bool {
    body.contains(&format!("class=\"nm\">{name}</div>"))
}

#[tokio::test]
async fn dashboard_mixed_statuses_populate_the_merged_tiles() {
    let server = server_with_mixed_statuses().await;
    let body = server.get("/").await.text();
    // Up folds in the running check; late and down stand alone; no Running tile.
    assert_tile(&body, "Total", 4);
    assert_tile(&body, "Up", 2);
    assert_tile(&body, "Late", 1);
    assert_tile(&body, "Down", 1);
    assert!(
        !body.contains(">Running</div>"),
        "there must be no Running tile: {body}"
    );
    for name in ["web", "job", "cron", "db"] {
        assert!(
            shows_check(&body, name),
            "{name} row missing unfiltered: {body}"
        );
    }
}

#[tokio::test]
async fn dashboard_status_filter_narrows_the_list_but_not_the_tiles() {
    let server = server_with_mixed_statuses().await;
    let body = server.get("/?status=down").await.text();

    // Only the down check is listed...
    assert!(shows_check(&body, "db"), "down check missing: {body}");
    for name in ["web", "job", "cron"] {
        assert!(
            !shows_check(&body, name),
            "{name} must be hidden by status=down: {body}"
        );
    }
    // ...but the tiles still show the full breakdown, so the other buckets
    // remain visible to switch to. This is the deliberate exception to
    // "counters follow the filter": they follow `q`, not the status select.
    assert_tile(&body, "Total", 4);
    assert_tile(&body, "Up", 2);
    assert_tile(&body, "Late", 1);
    assert_tile(&body, "Down", 1);
    // The select re-selects the active option, and the clear affordance shows.
    assert!(
        body.contains("value=\"down\" selected"),
        "the status select must re-select the active option: {body}"
    );
    assert!(
        body.contains("dashboard-filter-clear"),
        "an active status filter must offer a way out: {body}"
    );
}

/// The merge again, this time in the filter: `status=up` must include the
/// in-flight running check, not just the plain-up one.
#[tokio::test]
async fn dashboard_status_up_filter_includes_running_checks() {
    let server = server_with_mixed_statuses().await;
    let body = server.get("/?status=up").await.text();

    assert!(shows_check(&body, "web"), "plain-up check missing: {body}");
    assert!(
        shows_check(&body, "job"),
        "running check must match status=up (up and running share a bucket): {body}"
    );
    for name in ["cron", "db"] {
        assert!(
            !shows_check(&body, name),
            "{name} must not match status=up: {body}"
        );
    }
}

#[tokio::test]
async fn dashboard_status_and_text_filters_combine_with_and() {
    let server = server_with_mixed_statuses().await;
    // `job` is up-bucket but its name doesn't contain "web"; `web` matches both.
    let body = server.get("/?q=web&status=up").await.text();

    assert!(
        shows_check(&body, "web"),
        "web should match q and status: {body}"
    );
    assert!(
        !shows_check(&body, "job"),
        "job is up but fails the text filter, so AND must exclude it: {body}"
    );
    assert!(!shows_check(&body, "db"), "db matches neither: {body}");
}

/// An unrecognised `?status=` value degrades to "no filter" — the full list,
/// no selected option, no clear link — rather than a 400 or an empty page.
#[tokio::test]
async fn dashboard_unknown_status_value_is_ignored() {
    let server = server_with_mixed_statuses().await;
    let body = server.get("/?status=bogus").await.text();

    for name in ["web", "job", "cron", "db"] {
        assert!(shows_check(&body, name), "{name} missing: {body}");
    }
    assert!(
        !body.contains("dashboard-filter-clear"),
        "a bogus status is not an active filter: {body}"
    );
    // A bogus value collapses to "All": the All option is selected, and none of
    // the real status options are.
    assert!(
        body.contains("value=\"\" selected"),
        "the All option should be selected for a bogus status: {body}"
    );
    for v in ["up", "late", "down"] {
        assert!(
            !body.contains(&format!("value=\"{v}\" selected")),
            "no real status option should be selected for a bogus status: {body}"
        );
    }
}

/// A status filter that matches nothing is the no-results state, not the
/// "no projects yet" state — even though `q` is empty.
#[tokio::test]
async fn dashboard_status_filter_with_no_matches_shows_no_results_not_empty() {
    let (server, store, uid) = logged_in_server().await;
    // A single project with one never-pinged ("new") check: nothing is down.
    let pid = store
        .create_project(uid, "svc", "", None, None, chrono::Utc::now())
        .await
        .unwrap();
    store
        .create_check(&pingward::store::NewCheck {
            project_id: pid,
            name: "fresh",
            ping_uuid: "cu-fresh",
            kind: pingward::models::ScheduleKind::Period,
            period_secs: Some(3600),
            grace_secs: 300,
            timezone: "UTC",
            ..Default::default()
        })
        .await
        .unwrap();

    let body = server.get("/?status=down").await.text();
    assert!(
        body.contains("dashboard-no-results"),
        "no-results state missing: {body}"
    );
    assert!(
        !body.contains("dashboard-empty"),
        "a user who owns projects must not be told they have none: {body}"
    );
    // The clear link must show even though only the status filter is active.
    assert!(
        body.contains("dashboard-filter-clear"),
        "a status-only filter must still offer clear: {body}"
    );
}

#[tokio::test]
async fn dashboard_empty_state_when_no_projects() {
    let (server, _store, _uid) = logged_in_server().await;
    let res = server.get("/").await;
    res.assert_status_ok();
    let body = res.text();
    assert!(
        body.contains("No projects yet"),
        "empty-state message missing"
    );
}

/// Names in the order the dashboard renders them. Tests assert the whole
/// sequence rather than one pairwise comparison, so a sort that happens to move
/// the pair being checked but scrambles the rest still fails.
fn rendered_order(body: &str, open: &str, close: &str) -> Vec<String> {
    body.split(open)
        .skip(1)
        .filter_map(|s| s.split_once(close))
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Project group headers, top to bottom. `<h2>` is used nowhere else in the
/// dashboard or its base template.
fn project_order(body: &str) -> Vec<String> {
    rendered_order(body, "<h2>", "</h2>")
}

/// Check rows, top to bottom, flattened across groups. Matches the row's `nm`
/// cell, so the search box and project headers cannot contribute.
fn check_order(body: &str) -> Vec<String> {
    rendered_order(body, "class=\"nm\">", "</div>")
}

#[tokio::test]
async fn dashboard_orders_projects_by_name_not_creation() {
    // The fixture creates "web" before "infra", so id order and name order
    // disagree — a dashboard still on id order fails here.
    let server = server_with_two_projects().await;
    let body = server.get("/").await.text();
    assert_eq!(
        project_order(&body),
        ["infra", "web"],
        "projects must be ordered by name: {body}"
    );
}

#[tokio::test]
async fn dashboard_project_order_by_name_is_case_insensitive() {
    let (server, store, uid) = logged_in_server().await;
    let now = chrono::Utc::now();
    // Byte order would put every uppercase name ahead of every lowercase one,
    // splitting the list on case instead of reading alphabetically.
    for name in ["Zulu", "alpha", "Mike"] {
        store
            .create_project(uid, name, "", None, None, now)
            .await
            .unwrap();
    }
    let body = server.get("/").await.text();
    assert_eq!(
        project_order(&body),
        ["alpha", "Mike", "Zulu"],
        "project name order must ignore case: {body}"
    );
}

/// Four checks in one project whose creation order is deliberately the reverse
/// of their activity order, plus one that has never been pinged.
async fn server_with_staggered_activity() -> TestServer {
    use pingward::models::CheckStatus;
    let (server, store, uid) = logged_in_server().await;
    let now = chrono::Utc::now();
    let pid = store
        .create_project(uid, "services", "", None, None, now)
        .await
        .unwrap();
    async fn mk(store: &Store, pid: i64, name: &str, uuid: &str) -> i64 {
        store
            .create_check(&pingward::store::NewCheck {
                project_id: pid,
                name,
                ping_uuid: uuid,
                kind: pingward::models::ScheduleKind::Period,
                period_secs: Some(3600),
                grace_secs: 300,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap()
    }
    let ago = |s: i64| now - chrono::Duration::seconds(s);

    // Created first, pinged longest ago — must render last of the pinged rows.
    let stale = mk(&store, pid, "stale", "cu-stale").await;
    store
        .mark_ping(stale, CheckStatus::Up, Some(ago(900)), None, None)
        .await
        .unwrap();
    // Never pinged: no ping and no start, so it sorts below everything.
    mk(&store, pid, "untouched", "cu-untouched").await;
    let middle = mk(&store, pid, "middle", "cu-middle").await;
    store
        .mark_ping(middle, CheckStatus::Up, Some(ago(600)), None, None)
        .await
        .unwrap();
    // In flight: only a start, no finish. The start is what dates it, and it is
    // the most recent activity of the four.
    let running = mk(&store, pid, "running", "cu-running").await;
    store
        .mark_ping(running, CheckStatus::New, None, Some(ago(60)), None)
        .await
        .unwrap();
    // Started long ago but finished recently — the finish wins, putting it
    // second. Guards against a sort that reads only `last_start_at`.
    let finished = mk(&store, pid, "finished", "cu-finished").await;
    store
        .mark_ping(
            finished,
            CheckStatus::Up,
            Some(ago(120)),
            Some(ago(1200)),
            None,
        )
        .await
        .unwrap();
    server
}

#[tokio::test]
async fn dashboard_orders_checks_by_most_recent_activity() {
    let server = server_with_staggered_activity().await;
    let body = server.get("/").await.text();
    assert_eq!(
        check_order(&body),
        ["running", "finished", "middle", "stale", "untouched"],
        "checks must be ordered by their last ping or start, newest first: {body}"
    );
}

#[tokio::test]
async fn dashboard_never_pinged_checks_keep_creation_order_at_the_bottom() {
    let (server, store, uid) = logged_in_server().await;
    let now = chrono::Utc::now();
    let pid = store
        .create_project(uid, "services", "", None, None, now)
        .await
        .unwrap();
    // No check has any activity, so every sort key ties and the order must fall
    // back to creation — not to whatever the sort happens to do with equal keys.
    for (name, uuid) in [("zeta", "cu-z"), ("alpha", "cu-a"), ("mu", "cu-m")] {
        store
            .create_check(&pingward::store::NewCheck {
                project_id: pid,
                name,
                ping_uuid: uuid,
                kind: pingward::models::ScheduleKind::Period,
                period_secs: Some(3600),
                grace_secs: 300,
                timezone: "UTC",
                ..Default::default()
            })
            .await
            .unwrap();
    }
    let body = server.get("/").await.text();
    assert_eq!(
        check_order(&body),
        ["zeta", "alpha", "mu"],
        "never-pinged checks must stay in creation order: {body}"
    );
}

#[tokio::test]
async fn dashboard_check_order_survives_the_status_filter() {
    // Filtering preserves relative order, so the newest-first sequence must
    // still hold once rows are removed.
    let server = server_with_staggered_activity().await;
    let body = server.get("/?status=up").await.text();
    assert_eq!(
        check_order(&body),
        ["running", "finished", "middle", "stale"],
        "the status filter must not scramble the activity order: {body}"
    );
}
