//! The one error type handlers return. It decides what the user sees.

use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// No (valid) session. Should send the user to `/login`.
    #[error("not logged in")]
    Unauthenticated,

    /// A rule in `policy` said no: name already owned, not your group, reserved name, …
    /// The message is safe to show to the user.
    #[error(transparent)]
    Denied(#[from] crate::policy::Denied),

    #[error("not found")]
    NotFound,

    /// NetBird failed or refused. Details are for the log, not the user.
    #[error("upstream netbird error: {0}")]
    Netbird(#[from] crate::netbird::Error),

    #[error(transparent)]
    Db(#[from] sqlx::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // TODO:
        //   Unauthenticated -> redirect to /login
        //   Denied          -> 403/422 with an inline error fragment (components::ui::notice),
        //                      retargeted with HX-Retarget so htmx shows it next to the form
        //   NotFound        -> 404
        //   anything else   -> log with tracing::error!, generic 500
        todo!()
    }
}
