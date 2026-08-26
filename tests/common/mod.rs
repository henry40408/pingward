//! Helpers shared between integration-test binaries.

/// Fixed session/CSRF secret for tests, pinned via [`test_config`] so helpers
/// can derive tokens without threading a `Config` through every call site.
/// Tests covering secret *rotation* build their own `Config` instead.
#[allow(dead_code)]
pub const TEST_SECRET: &str = "pingward-test-secret-32-bytes-xx";

/// A default `Config` pinned to [`TEST_SECRET`].
///
/// `#[allow(dead_code)]`: see [`substitute_owner_id`].
#[allow(dead_code)]
pub fn test_config() -> pingward::config::Config {
    pingward::config::Config::from_map(|k| (k == "PINGWARD_SECRET").then(|| TEST_SECRET.into()))
}

/// The one-shot flash cookie (`name=value`) as a [`test_config`] server sets
/// it: signed under [`TEST_SECRET`], unprefixed name (`cookie_secure` is false
/// without an `https://` `PINGWARD_BASE_URL`).
///
/// Writing `pingward_flash=<surface>` directly exercises nothing — the server
/// ignores unsigned values, as it must for one planted by a sibling subdomain.
#[allow(dead_code)]
pub fn signed_flash_cookie(value: &str) -> String {
    format!(
        "pingward_flash={}",
        pingward::secret::sign_flash(TEST_SECRET.as_bytes(), value)
    )
}

/// The payload a `Set-Cookie` flash value carries, verified under
/// [`TEST_SECRET`]. `raw` is the cookie's value, signature included.
#[allow(dead_code)]
pub fn flash_payload(raw: &str) -> Option<String> {
    pingward::secret::verify_flash(TEST_SECRET.as_bytes(), raw)
}

/// The CSRF token for the newest session row in `pool`, derived as a
/// [`test_config`] server derives it: `HMAC(secret, session id)`, with no
/// `sessions.csrf_token` column. "Newest" is by `rowid` — sessions in a test
/// share a second, so `created_at`/`expires_at` cannot order two rows apart.
#[allow(dead_code)]
pub async fn newest_session_csrf(pool: &pingward::db::Pool) -> String {
    let id = sqlx::query_scalar::<_, String>("SELECT id FROM sessions ORDER BY rowid DESC LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("a session row exists");
    pingward::secret::derive_csrf(TEST_SECRET.as_bytes(), &id)
}

/// Starts a fresh anonymous session on `server` and returns its CSRF token, for
/// the `_csrf` field of a `POST /login` or `POST /setup`. Neither is exempt
/// from `csrf_guard`, and `TestServer` posts blind where a browser would first
/// load the form carrying the hidden `_csrf`.
///
/// The token comes from the *cookie*: an anonymous session has no `sessions`
/// row, so [`newest_session_csrf`] cannot see it — use this before logging in
/// and that one after. Cookies are cleared first so the layer is guaranteed to
/// mint on this request, since `TestServer` exposes no reader for its jar.
/// Both cookie names are tried; a handful of callers build a `Secure`
/// (`__Host-`-prefixed) server.
///
/// `#[allow(dead_code)]`: see [`substitute_owner_id`].
#[allow(dead_code)]
pub async fn anonymous_csrf(server: &mut axum_test::TestServer) -> String {
    server.clear_cookies();
    let res = server.get("/login").await;
    let cookie = res
        .maybe_cookie(pingward::auth::session_cookie_name(true))
        .or_else(|| res.maybe_cookie(pingward::auth::session_cookie_name(false)))
        .expect("the anonymous-session layer sets a cookie on a session-less request");
    let id = pingward::secret::verify_session(TEST_SECRET.as_bytes(), cookie.value())
        .expect("the anonymous cookie is signed with the test secret");
    pingward::secret::derive_csrf(TEST_SECRET.as_bytes(), &id)
}

/// Recovers every `(method, path)` pair a router registers by parsing the body
/// of its `pub fn routes() -> Router<AppState> {` out of the source text,
/// keeping those starting with `prefix`. `axum::Router` exposes no route table
/// at runtime, so the source is the only alternative to a hand-maintained copy.
/// Plain `str` methods only — no regex crate.
///
/// Paths come back raw (`{param}` segments intact) for callers that need to
/// know which segment carries an id; run them through [`normalise_route_path`]
/// to request them as-is. `source` is the file text — callers pass
/// `include_str!(...)` so the path resolves relative to their own file.
///
/// `#[allow(dead_code)]`: see [`substitute_owner_id`].
#[allow(dead_code)]
pub fn routes_in_router_source(source: &str, prefix: &str) -> Vec<(&'static str, String)> {
    let start_marker = "pub fn routes() -> Router<AppState> {";
    let start = source
        .find(start_marker)
        .expect("source: `pub fn routes()` not found")
        + start_marker.len();
    let rest = &source[start..];
    let body_end = rest
        .find("\n}\n")
        .expect("source: end of routes() body not found");
    let body = &rest[..body_end];

    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(rel) = body[pos..].find(".route(") {
        let entry_start = pos + rel + ".route(".len();
        let entry_end = body[entry_start..]
            .find(".route(")
            .map_or(body.len(), |r| entry_start + r);
        let entry = &body[entry_start..entry_end];
        pos = entry_end;

        let q1 = entry.find('"').expect("route entry missing path literal");
        let q2 = entry[q1 + 1..]
            .find('"')
            .expect("route entry: unterminated path literal")
            + q1
            + 1;
        let raw_path = &entry[q1 + 1..q2];
        if !raw_path.starts_with(prefix) {
            continue;
        }
        let path = raw_path.to_string();
        let mut methods = 0;
        if entry.contains("get(") {
            out.push(("GET", path.clone()));
            methods += 1;
        }
        if entry.contains("post(") {
            out.push(("POST", path.clone()));
            methods += 1;
        }
        if entry.contains("put(") {
            out.push(("PUT", path.clone()));
            methods += 1;
        }
        if entry.contains("patch(") {
            out.push(("PATCH", path.clone()));
            methods += 1;
        }
        if entry.contains("delete(") {
            out.push(("DELETE", path));
            methods += 1;
        }
        assert!(
            methods > 0,
            "route `{raw_path}` uses a method router this parser doesn't recognise \
             (only `get(`/`post(`/`put(`/`patch(`/`delete(` are handled) — extend \
             `routes_in_router_source` so the route stays covered"
        );
    }
    out
}

/// Substitutes a raw route's first `{param}` segment with the id of the
/// resource named by the segment before it — `projects` takes `project_id`,
/// `checks` takes `check_id`, `channels` takes `channel_id`. Panics on an
/// unrecognised resource segment so a future route with a new resource type
/// fails loudly instead of being silently mis-targeted.
///
/// `#[allow(dead_code)]`: each `tests/*.rs` binary compiles its own copy of
/// this module, so a function no call site in *that* binary uses reads as dead
/// there.
#[allow(dead_code)]
pub fn substitute_owner_id(
    raw_path: &str,
    project_id: i64,
    check_id: i64,
    channel_id: i64,
) -> String {
    let start = raw_path
        .find('{')
        .unwrap_or_else(|| panic!("route `{raw_path}` has no `{{param}}` segment to substitute"));
    let end = raw_path[start..].find('}').map_or_else(
        || panic!("route `{raw_path}` has an unterminated `{{param}}` segment"),
        |rel| start + rel + 1,
    );
    let segment = raw_path[..start].trim_end_matches('/').rsplit('/').next();
    let id = match segment {
        Some("projects") => project_id,
        Some("checks") => check_id,
        Some("channels") => channel_id,
        other => panic!(
            "route `{raw_path}`: unrecognised resource segment {other:?} before its path \
             parameter — add a case to `substitute_owner_id` for this resource type"
        ),
    };
    format!("{}{}{}", &raw_path[..start], id, &raw_path[end..])
}

/// Replaces every `{param}` path segment with `1` so the parsed path can be
/// requested as-is.
///
/// `#[allow(dead_code)]`: see [`substitute_owner_id`].
#[allow(dead_code)]
pub fn normalise_route_path(raw: &str) -> String {
    let mut out = String::new();
    let mut in_param = false;
    for c in raw.chars() {
        match c {
            '{' => {
                in_param = true;
                out.push('1');
            }
            '}' => in_param = false,
            _ if in_param => {}
            _ => out.push(c),
        }
    }
    out
}

/// Unlock `/admin`'s access-granting controls for this server's session.
///
/// Creating a user, resetting a password and granting admin hand out access
/// outliving the browser session, so they sit behind `elevate::Elevations` and
/// need the admin's password re-asserted first. Requires the session's CSRF
/// token to already be installed as a default header.
///
/// `#[allow(dead_code)]`: see [`substitute_owner_id`].
#[allow(dead_code)]
pub async fn unlock_admin(server: &axum_test::TestServer, password: &str) {
    server
        .post("/admin/unlock")
        .form(&[("password", password)])
        .await
        .assert_status(axum::http::StatusCode::SEE_OTHER);
}
