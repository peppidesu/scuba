//! The ownership rules, as pure functions. No I/O and no `async`, so they are easy to read and
//! to test exhaustively: `services` loads the facts from the database, asks this module whether
//! an action is allowed, and only then changes anything.
//!
//! # The rules
//!
//! **DNS names** — a user can obtain DNS names in two ways:
//! * claim a **TLD** (e.g. `corp`), which gives them the name and everything beneath it, or
//! * claim a **domain under `.reef`** (e.g. `foo.reef`), likewise with everything beneath it.
//!
//! Nobody can claim a name that overlaps one owned by someone else: the same name, a parent of
//! it, or a child of it. `reef` itself can't be claimed as a TLD. Users can only create, change
//! or remove records beneath names they own.
//!
//! **Groups** — anyone can create a group that doesn't exist yet and becomes its owner. The
//! owner of a group decides which other groups may send traffic *to* it. This is one-way:
//! owning a group gives no say over what its members may reach.
//!
//! **`reef`** is the public group. Every machine can reach it (not the other way around), and
//! users may always put their own devices in it to expose them to everyone.
//!
//! **Devices** — users see and manage only their own: remove them, or change which groups
//! they're in.
//!
//! Open question: may a user put a device into a group they don't own? As things stand
//! [`groups::can_join`] only allows `reef` and groups the user owns. Allowing others' groups
//! would let anyone join a group whose owner has opened it up to some other group.

pub mod dns;
pub mod groups;

/// Name of the system-owned public group that every machine can reach.
pub const REEF_GROUP: &str = "reef";

/// The TLD under which per-user domains such as `foo.reef` live.
pub const REEF_TLD: &str = "reef";

/// Why an action was refused. The messages are written for the person who asked, so they can
/// be shown as-is.
#[derive(Debug, thiserror::Error)]
pub enum Denied {
    /// Someone else already owns this name (or one that overlaps it).
    #[error("`{0}` is already owned by another user")]
    AlreadyOwned(String),

    /// A name only the system may use, such as `reef` as a TLD or as a group name.
    #[error("`{0}` is reserved")]
    Reserved(String),

    /// Not a valid DNS name or group name; the string says what's wrong.
    #[error("invalid name: {0}")]
    InvalidName(String),

    /// The user tried to manage something that belongs to someone else.
    #[error("you do not own `{0}`")]
    NotOwner(String),

    /// Anything else that's not allowed.
    #[error("not allowed: {0}")]
    Other(&'static str),
}
