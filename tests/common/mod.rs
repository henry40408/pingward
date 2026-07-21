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
        let path = normalise_route_path(raw_path);
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

/// Replaces every `{param}` path segment with `1` so the parsed path can be
/// requested as-is.
fn normalise_route_path(raw: &str) -> String {
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
