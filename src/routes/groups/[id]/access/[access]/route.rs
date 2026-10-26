//! `/groups/{id}/access/{access}`

use axum::extract::{Path, State};

use crate::{auth::AuthSession, error::AppError, state::AppState};

/// `DELETE /groups/{id}/access/{access}` — take back a rule allowing traffic into group `{id}`.
/// Answers with an empty body, which htmx uses to remove the table row.
pub async fn delete(
    State(_state): State<AppState>,
    _auth: AuthSession,
    Path((_group, _access)): Path<(i64, i64)>,
) -> Result<(), AppError> {
    todo!()
}
