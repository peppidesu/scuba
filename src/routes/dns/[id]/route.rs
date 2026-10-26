//! `/dns/{id}`

use axum::extract::{Path, State};

use crate::{auth::AuthSession, error::AppError, state::AppState};

/// `DELETE /dns/{id}` — release one of the user's claims, deleting the records beneath it.
/// Answers with an empty body, which htmx uses to remove the table row.
pub async fn delete(
    State(_state): State<AppState>,
    _auth: AuthSession,
    Path(_id): Path<i64>,
) -> Result<(), AppError> {
    todo!()
}
