//! The step definitions, one module per `steps/*.steps.js` the JavaScript
//! suite had.
//!
//! Cucumber collects `#[given]` / `#[when]` / `#[then]` attributes at link
//! time, so a module only has to be reachable from the crate root to
//! contribute its steps; nothing here is called directly.

pub mod account;
pub mod admin;
pub mod assets;
pub mod auth;
pub mod authz;
pub mod check_create;
pub mod check_history;
pub mod edit_flows;
pub mod live_tail;
pub mod mobile_layout;
pub mod monitoring;
pub mod no_js;
pub mod notifications;
pub mod ping_kinds;
pub mod settings;
pub mod theme;
pub mod time_states;
pub mod users;
pub mod validation;
