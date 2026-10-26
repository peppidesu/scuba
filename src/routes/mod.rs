//! The HTTP handlers, laid out as files: the folder structure *is* the URL structure
//! (via `axum-folder-router`). Each `route.rs` exports `get`, `post`, `put` and/or `delete`
//! functions, one per HTTP method.
//!
//! ```text
//! routes/route.rs                              GET     /                          → redirect to /devices
//! routes/devices/route.rs                      GET     /devices
//! routes/devices/[id]/route.rs                 DELETE  /devices/{id}
//! routes/devices/[id]/groups/route.rs          PUT     /devices/{id}/groups
//! routes/dns/route.rs                          GET     /dns                       list claims
//!                                              POST    /dns                       claim a name
//! routes/dns/[id]/route.rs                     DELETE  /dns/{id}                  release a claim
//! routes/dns/records/route.rs                  POST    /dns/records
//! routes/dns/records/[id]/route.rs             DELETE  /dns/records/{id}
//! routes/groups/route.rs                       GET     /groups
//!                                              POST    /groups                    create a group
//! routes/groups/[id]/access/route.rs           POST    /groups/{id}/access        allow incoming traffic
//! routes/groups/[id]/access/[access]/route.rs  DELETE  /groups/{id}/access/{access}
//! ```
//!
//! Handlers stay thin: extract the request, call one function in `services`, render the result
//! with `components`.
//!
//! **Gotcha:** a `[id]` folder becomes a path parameter literally named `:id`, so extract path
//! parameters by position (`Path<i64>`, `Path<(i64, i64)>`), never into a struct with named fields.
//!
//! Everything here requires a logged-in user. The public routes (`/login`, `/auth/…`) are
//! in [`crate::auth`].

use axum::Router;
use axum_folder_router::folder_router;
use axum_login::login_required;

use crate::{auth::Backend, state::AppState};

#[folder_router("src/routes", AppState)]
struct Routes();

/// All the routes above, behind `login_required!` (anonymous visitors go to `/login`).
pub fn router() -> Router<AppState> {
    Routes::into_router().route_layer(login_required!(Backend, login_url = "/login"))
}
