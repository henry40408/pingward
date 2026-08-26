//! Browser E2E support for pingward: the server under test, the browser
//! session the Cucumber steps drive, the mock receiver the notification
//! scenarios assert against, and the seed behind the README screenshots.

pub mod actions;
pub mod api;
pub mod browser;
pub mod dom;
pub mod mock;
pub mod seed;
pub mod server;
pub mod wait;
pub mod world;

pub use api::{Api, PingKind};
pub use server::Server;

/// Undoes the backslash escaping a Gherkin `{string}` argument keeps.
///
/// cucumber-rs passes the raw capture, backslashes and all, so
/// `"unknown timezone \"Asia/Taipeh\""` would be compared against text no page
/// renders. Only the two escapes Gherkin defines are recognised, so a Windows
/// path or a regex in a step argument survives intact.
pub fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some(escaped @ ('"' | '\\')) => out.push(escaped),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}
