//! A user's own devices (NetBird peers).

use crate::{db::users::User, error::AppError, netbird::Peer, state::AppState};

/// The user's devices: peers whose `user_id` is the user's NetBird user.
pub async fn list(_state: &AppState, _user: &User) -> Result<Vec<Peer>, AppError> {
    todo!()
}

/// Remove one of the user's devices. Checks that the peer really is theirs first.
pub async fn remove(_state: &AppState, _user: &User, _peer_id: &str) -> Result<(), AppError> {
    todo!()
}

/// Set which groups one of the user's devices belongs to (by group name).
/// Only groups the user may join are changed; see [`crate::policy::groups::can_join`].
pub async fn set_groups(
    _state: &AppState,
    _user: &User,
    _peer_id: &str,
    _group_names: Vec<String>,
) -> Result<(), AppError> {
    todo!()
}
