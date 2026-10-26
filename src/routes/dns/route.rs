//! `/dns`

use axum::extract::{Form, State};
use maud::Markup;

use crate::{auth::AuthSession, error::AppError, state::AppState};

/// The "claim a name" form.
// Fields are read once the handler is implemented.
#[allow(dead_code)]
#[derive(serde::Deserialize)]
pub struct Claim {
    /// `"tld"` or `"reef"` (see `policy::dns::ClaimKind`).
    pub kind: String,
    /// What the user typed; for `reef` only the part before `.reef`.
    pub name: String,
}

/// `GET /dns` — the user's claims and the records beneath them.
pub async fn get(State(_state): State<AppState>, _auth: AuthSession) -> Result<Markup, AppError> {
    todo!()
}

/// `POST /dns` — claim a TLD or a `.reef` domain. Answers with the new claim's row, or with an
/// error message if the name is taken.
pub async fn post(
    State(_state): State<AppState>,
    _auth: AuthSession,
    Form(_claim): Form<Claim>,
) -> Result<Markup, AppError> {
    todo!()
}
