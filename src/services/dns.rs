//! DNS names a user owns, and the records beneath them.

use crate::{
    db::{dns::DnsClaim, users::User},
    error::AppError,
    policy::dns::ClaimKind,
    state::AppState,
};

/// The user's own claims.
pub async fn list_claims(_state: &AppState, _user: &User) -> Result<Vec<DnsClaim>, AppError> {
    todo!()
}

/// Claim a TLD or a `.reef` domain. A TLD gets its own zone in NetBird; a `.reef` domain is
/// just a record scope inside the shared `reef` zone.
pub async fn claim(
    _state: &AppState,
    _user: &User,
    _kind: ClaimKind,
    _name: &str,
) -> Result<DnsClaim, AppError> {
    todo!()
}

/// Give a claim up, deleting its records (and its zone, for a TLD).
pub async fn release(_state: &AppState, _user: &User, _claim_id: i64) -> Result<(), AppError> {
    todo!()
}

/// Create or update a record. `fqdn` must be beneath one of the user's claims.
pub async fn put_record(
    _state: &AppState,
    _user: &User,
    _fqdn: &str,
    _rtype: &str,
    _content: &str,
) -> Result<(), AppError> {
    todo!()
}

/// Remove a record. Checks that it is beneath one of the user's claims.
pub async fn delete_record(
    _state: &AppState,
    _user: &User,
    _record_id: i64,
) -> Result<(), AppError> {
    todo!()
}
