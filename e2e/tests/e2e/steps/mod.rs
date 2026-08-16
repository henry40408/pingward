//! The step definitions, one module per `steps/*.steps.js` the JavaScript
//! suite had.
//!
//! Cucumber collects `#[given]` / `#[when]` / `#[then]` attributes at link
//! time, so a module only has to be reachable from the crate root to
//! contribute its steps; nothing here is called directly.

pub mod auth;
pub mod check_create;
pub mod edit_flows;
pub mod monitoring;
pub mod settings;
pub mod validation;
