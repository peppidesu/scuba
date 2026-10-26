//! `/dns/records`

use axum::extract::{Form, State};
use maud::Markup;

use crate::{auth::AuthSession, error::AppError, state::AppState};

/// The "add a record" form.
// Fields are read once the handler is implemented.
#[allow(dead_code)]
#[derive(serde::Deserialize)]
pub struct Record {
    pub fqdn: String,
    /// `A`, `AAAA`, `CNAME`, …
    pub rtype: String,
    pub content: String,
}

/// `POST /dns/records` — add a record beneath one of the user's claims.
/// Answers with the new record's row, or with an error message.
pub async fn post(
    State(_state): State<AppState>,
    _auth: AuthSession,
    Form(_record): Form<Record>,
) -> Result<Markup, AppError> {
    todo!()
}
