//! `/devices/{id}/groups`

use axum::extract::{Form, Path, State};
use maud::Markup;

use crate::{auth::AuthSession, error::AppError, state::AppState};

/// The submitted checkboxes: one `groups=<name>` entry per ticked group.
// Fields are read once the handler is implemented.
#[allow(dead_code)]
#[derive(serde::Deserialize)]
pub struct Body {
    pub groups: Vec<String>,
}

/// `PUT /devices/{id}/groups` — set which groups one of the user's devices is in.
/// Answers with the device's updated row.
pub async fn put(
    State(_state): State<AppState>,
    _auth: AuthSession,
    Path(_id): Path<String>,
    Form(_body): Form<Body>,
) -> Result<Markup, AppError> {
    todo!()
}
