//! `/devices`

use axum::extract::State;
use maud::Markup;

use crate::{auth::AuthSession, error::AppError, state::AppState};

/// `GET /devices` — the logged-in user's devices, with their groups.
/// Renders the whole page, or just the table when htmx asks for a fragment.
pub async fn get(State(_state): State<AppState>, _auth: AuthSession) -> Result<Markup, AppError> {
    todo!()
}
