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

/// Undoes the backslash escaping cucumber-rs leaves in a `{string}` argument.
/// Only `\"` and `\\` are recognised, so other backslashes survive intact.
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
