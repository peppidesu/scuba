//! Group rules: who may create, join and open up groups.
//! See the module docs of [`crate::policy`] for the rules in words.

use super::Denied;
use crate::db::groups::Group;

/// Is this an acceptable group name? Rejects invalid characters and `reef` (system-owned).
pub fn validate_name(_name: &str) -> Result<(), Denied> {
    todo!()
}

/// May `user` allow traffic *into* `target`? Only its owner may. `reef` has no owner,
/// so nobody can change what is allowed into it.
pub fn can_allow_into(_user: i64, _target: &Group) -> Result<(), Denied> {
    todo!()
}

/// May `user` put one of their devices into `group`? Currently: `reef`, or a group they own.
/// (See the open question in the [`crate::policy`] docs.)
pub fn can_join(_user: i64, _group: &Group) -> Result<(), Denied> {
    todo!()
}
