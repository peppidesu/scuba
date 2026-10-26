//! scuba: a small web app for managing your own corner of a NetBird mesh VPN.
//!
//! Users log in through authentik, then can manage their devices, claim DNS names, and own
//! groups with access rules. NetBird is the source of truth for the VPN itself; scuba's own
//! database only records *who owns what*.
//!
//! # How the code is organised
//!
//! A request flows top to bottom through these layers; each only talks to the ones below it.
//!
//! | Module       | Role                                                                    |
//! |--------------|-------------------------------------------------------------------------|
//! | [`routes`]   | HTTP handlers, one `route.rs` per URL (file-based routing)              |
//! | [`components`] | HTML building blocks (maud), page shells, CSS/JS assets               |
//! | [`services`] | One function per user action: load facts → check rules → change things |
//! | [`policy`]   | The ownership rules, as pure functions with no I/O                      |
//! | [`db`]       | Typed SQL queries (sqlite)                                              |
//! | [`netbird`]  | Client for NetBird's management API                                     |
//! | [`auth`]     | Login with authentik (OIDC) and sessions                                |
//! | [`config`], [`state`], [`error`] | Settings from the environment, shared state, error type |
//!
//! `src/main.rs` wires it together. For running everything locally see `dev/README.md`.

pub mod auth;
pub mod components;
pub mod config;
pub mod db;
pub mod error;
pub mod netbird;
pub mod policy;
pub mod routes;
pub mod services;
pub mod state;
