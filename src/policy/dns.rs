//! DNS rules: which names a user may claim, and which records they may manage.
//! See the module docs of [`crate::policy`] for the rules in words.

use super::Denied;
use crate::db::dns::DnsClaim;

/// The two ways of claiming a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimKind {
    /// A top-level domain such as `corp`.
    Tld,
    /// A domain directly under `.reef`, such as `foo.reef`.
    Reef,
}

/// Validate and normalise what the user typed: lowercase, no trailing dot, valid labels.
///
/// For [`ClaimKind::Reef`] the input is only the part *before* `.reef` (the form shows
/// `.reef` as a fixed suffix) and the returned name includes it.
pub fn normalize(_kind: ClaimKind, _input: &str) -> Result<String, Denied> {
    todo!()
}

/// May `user` claim `name`?
///
/// Refused if any claim owned by *someone else* is equal to, a parent of, or a child of `name`
/// ([`Denied::AlreadyOwned`]); `reef` itself is refused as a TLD ([`Denied::Reserved`]).
/// `existing` must be every claim from every owner.
pub fn check_claim(_user: i64, _name: &str, _existing: &[DnsClaim]) -> Result<(), Denied> {
    todo!()
}

/// The claim of `user` that covers `fqdn`, i.e. the claim that makes it theirs to manage;
/// [`Denied::NotOwner`] if there is none. `owned` is the user's own claims.
pub fn covering_claim<'a>(
    _user: i64,
    _fqdn: &str,
    _owned: &'a [DnsClaim],
) -> Result<&'a DnsClaim, Denied> {
    todo!()
}
