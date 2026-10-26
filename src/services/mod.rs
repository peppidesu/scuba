//! One function per thing a user can do. Each follows the same recipe:
//!
//! 1. **load** the facts it needs (`db`, `netbird`),
//! 2. **check** the rules ([`crate::policy`]),
//! 3. **change** things: NetBird first, then our database.
//!
//! Handlers in `routes` call these and nothing else, and every function takes the acting
//! user, so no handler can forget whose request it is.
//!
//! Why NetBird before the database: if the second step fails we're left with a harmless
//! unrecorded object in NetBird that a later reconcile can clean up. The other way round we'd
//! record ownership of something that doesn't exist. When the database write fails, try to undo
//! the NetBird change.

pub mod devices;
pub mod dns;
pub mod groups;
