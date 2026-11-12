//! Server-rendered pages. Each page is a function from a view struct to `Markup`;
//! `layout::page` supplies the shared document.

pub mod account;
pub mod admin_unlock;
pub mod auth;
pub mod channel_form;
pub mod check;
pub mod check_form;
pub mod check_history;
pub mod confirm;
pub mod dashboard;
pub mod layout;
pub mod project;
