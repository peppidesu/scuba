//! `/groups`

use axum::extract::{Form, State};
use maud::Markup;

use crate::{auth::AuthSession, error::AppError, state::AppState};

/// The "create a group" form.
// Fields are read once the handler is implemented.
#[allow(dead_code)]
#[derive(serde::Deserialize)]
pub struct Create {
    pub name: String,
}

/// `GET /groups` — the groups the user owns, and who may send traffic into each.
pub async fn get(State(_state): State<AppState>, _auth: AuthSession) -> Result<Markup, AppError> {
    todo!()
}

/// `POST /groups` — create a group; the user becomes its owner. Answers with the new group's
/// row, or with an error message if the name exists.
pub async fn post(
    State(_state): State<AppState>,
    _auth: AuthSession,
    Form(_create): Form<Create>,
) -> Result<Markup, AppError> {
    todo!()
}
