//! Server-rendered pages. Each page is a function from a view struct to `Markup`;
//! `layout::page` supplies the shared document.

pub mod admin_unlock;
pub mod auth;
pub mod confirm;
pub mod dashboard;
pub mod layout;
pub mod project;
