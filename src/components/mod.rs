//! Shared maud fragments and static assets.
//!
//! - `layout`: page shells (`page`, `app_page`)
//! - `ui`: small composable pieces (tables, forms, buttons, notices)
//! - `assets`: CSS/JS served from the binary
//! - `styleguide`: debug-only kitchen-sink page

pub mod assets;
pub mod layout;
pub mod styleguide;
pub mod ui;

pub use layout::{Nav, app_page, page};
