//! `GET /` — there's no home page; send people to their devices.

use axum::response::Redirect;

pub async fn get() -> Redirect {
    Redirect::to("/devices")
}
