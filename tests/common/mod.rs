//! Helpers shared between integration-test binaries. This directory (a
//! `mod.rs` under `tests/common/`) is the standard Rust idiom for code shared
//! between `tests/*.rs` files without itself being compiled as a separate
//! test binary.

/// Parses the body of a router's `pub fn routes() -> Router<AppState> {`
/// function straight out of its own source to recover every `(method, path)`
/// pair it registers, filtered to those starting with `prefix`. This is a
/// deliberate source-level check: `axum::Router` does not expose its route
/// table for introspection at runtime, so reading the router's own source is
/// the only way to recover the list without hand-maintaining a copy of it.
/// Plain `str` methods only — no regex crate.
///
/// Paths are returned **raw** (`{param}` segments intact, not normalised) —
/// callers that just want to request the path as-is should run it through
/// [`normalise_route_path`] themselves; callers that need to know which
/// segment carries an id (e.g. to substitute a real one) need the raw form.
///
/// `source` is the file text; callers pass `include_str!(...)` at the call
/// site so the path resolves relative to the calling file. `prefix` selects
/// which registered paths to keep (e.g. `"/admin"`, `"/api/v1"`).
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
/// resource type named by the path segment immediately before it —
/// `.../projects/{id}...` gets `project_id`, `.../checks/{id}...` gets
/// `check_id`, `.../channels/{id}...` gets `channel_id`. Panics on an
/// unrecognised resource segment so a future route with a new resource type
/// fails loudly instead of being silently mis-targeted. Shared by the
/// `/api/v1` and web-surface ownership-scoping tests, which both route ids
/// through the same three resource types.
///
/// `#[allow(dead_code)]`: each `tests/*.rs` binary compiles its own copy of
/// this module, so rustc only sees the calls made from *that* binary — not
/// every function in `tests/common/` is used by every consumer (e.g.
/// `tests/admin.rs` has no cross-user id substitution to do), which would
/// otherwise be flagged as dead code in the binaries that don't call it.
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
/// `#[allow(dead_code)]`: see the note on [`substitute_owner_id`] — not every
/// `tests/*.rs` binary that pulls in this module calls every function in it.
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
