//! `/groups/{id}/access`

use axum::extract::{Form, Path, State};
use maud::Markup;

use crate::{auth::AuthSession, error::AppError, state::AppState};

/// The "allow incoming traffic" form.
// Fields are read once the handler is implemented.
#[allow(dead_code)]
#[derive(serde::Deserialize)]
pub struct Allow {
    /// Name of the group whose members may reach this group.
    pub source: String,
}

/// `POST /groups/{id}/access` — the owner of group `{id}` allows traffic from another group
/// into it. Answers with the new rule's row.
pub async fn post(
    State(_state): State<AppState>,
    _auth: AuthSession,
    Path(_id): Path<i64>,
    Form(_allow): Form<Allow>,
) -> Result<Markup, AppError> {
    todo!()
}
