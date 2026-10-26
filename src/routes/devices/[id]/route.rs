//! `/devices/{id}`

use axum::extract::{Path, State};

use crate::{auth::AuthSession, error::AppError, state::AppState};

/// `DELETE /devices/{id}` — remove one of the user's devices from the VPN.
/// Answers with an empty body, which htmx uses to remove the table row.
pub async fn delete(
    State(_state): State<AppState>,
    _auth: AuthSession,
    Path(_id): Path<String>,
) -> Result<(), AppError> {
    todo!()
}
