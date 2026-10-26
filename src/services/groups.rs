//! Groups, who owns them, and who may send traffic into them.

use crate::{
    db::{groups::Group, users::User},
    error::AppError,
    state::AppState,
};

/// The groups the user owns.
pub async fn list_owned(_state: &AppState, _user: &User) -> Result<Vec<Group>, AppError> {
    todo!()
}

/// Create a group and make the user its owner. Fails with
/// [`Denied::AlreadyOwned`](crate::policy::Denied::AlreadyOwned) if the name exists,
/// whoever owns it.
pub async fn create(_state: &AppState, _user: &User, _name: &str) -> Result<Group, AppError> {
    todo!()
}

/// The owner of `target` lets traffic from `source` (any existing group) flow into it.
/// One-way: this says nothing about traffic from `target` to `source`.
pub async fn allow_incoming(
    _state: &AppState,
    _user: &User,
    _target: &str,
    _source: &str,
) -> Result<(), AppError> {
    todo!()
}

/// Take such a rule away again.
pub async fn revoke_incoming(
    _state: &AppState,
    _user: &User,
    _access_id: i64,
) -> Result<(), AppError> {
    todo!()
}

/// Run at startup. Makes sure NetBird has everything scuba assumes about `reef`:
/// the `reef` group (no owner), the `reef` DNS zone, and a policy letting the built-in
/// `All` group (every peer) reach `reef`. Using `All` as the source means new groups need
/// no extra policy work.
pub async fn ensure_reef(_state: &AppState) -> Result<(), AppError> {
    todo!()
}
