//! The component kitchen sink as a standalone page, for working on the CSS and components:
//!
//! ```sh
//! cargo run --example styleguide   # then open http://127.0.0.1:3999/
//! ```
//!
//! No login, no database, no NetBird. The CSS is compiled in, so rebuild after editing it.

use axum::{Router, routing::get};
use scuba::components::{self, Nav, styleguide};
use scuba::db::users::User;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // A made-up user, just so the page frame has someone to show in the corner.
    let user = User {
        id: 1,
        oidc_sub: "demo".into(),
        email: "demo@example.com".into(),
        display_name: "demo".into(),
        netbird_user_id: None,
    };
    let app: Router = Router::new()
        .route(
            "/",
            get(move || async move {
                components::app_page(&user, Nav::Devices, "Styleguide", styleguide::body())
            }),
        )
        .merge(components::assets::router());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3999").await?;
    println!("http://{}", listener.local_addr()?);
    axum::serve(listener, app).await?;
    Ok(())
}
