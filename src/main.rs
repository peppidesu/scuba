//! scuba's entry point: read configuration, connect to the database and NetBird, build the
//! router, serve. All the real code lives in the library crate (`src/lib.rs`).

use axum::Router;
use tower_http::trace::TraceLayer;

use scuba::{auth, components, config, db, netbird, routes, services, state};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // ---- dependencies ----
    let cfg = config::Config::from_env()?;
    let db = db::connect(&cfg.database_url).await?;
    let netbird = netbird::NetbirdClient::new(cfg.netbird.api_url, cfg.netbird.api_token)?;
    let state = state::AppState {
        db: db.clone(),
        netbird,
    };

    // Make sure NetBird has the `reef` group, zone and policy that the rest of scuba assumes.
    services::groups::ensure_reef(&state).await?;

    // Expired sessions are only filtered out on read; this deletes them from time to time.
    tokio::spawn(db::sessions::cleanup_loop(
        db.clone(),
        std::time::Duration::from_secs(3600),
    ));

    // Needs authentik to be up: this does OIDC discovery.
    let backend = auth::Backend::new(db.clone(), &cfg.oidc, &cfg.public_url).await?;

    // ---- routes ----
    // TODO: require the `HX-Request` header on mutating routes (cheap CSRF defence on top of
    // the SameSite=Lax session cookie).
    let app: Router = Router::new()
        .merge(auth::router()) // public: /login, /auth/callback, /logout
        .merge(routes::router()) // everything else requires a logged-in user
        .with_state(state)
        .merge(components::assets::router()); // public: /static/…

    // The component kitchen sink, for development only.
    #[cfg(debug_assertions)]
    let app = app.merge(
        components::styleguide::router().route_layer(axum_login::login_required!(
            auth::Backend,
            login_url = "/login"
        )),
    );

    // ---- layers and serve ----
    let secure_cookies = cfg.public_url.starts_with("https://");
    let app =
        auth::with_auth_layers(app, db, backend, secure_cookies).layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(&cfg.listen).await?;
    tracing::info!("listening on {}", cfg.listen);
    axum::serve(listener, app).await?;
    Ok(())
}
