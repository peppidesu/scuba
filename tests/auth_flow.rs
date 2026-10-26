//! The full login flow, end to end: scuba's router (served in-process) talking to a fake
//! authentik (over HTTP). Each test starts its own fake IdP and in-memory database.
//!
//! Most tests follow the same shape: `/login` → the IdP → the callback, then check what happened.
//! See `tests/dev_stack.rs` for the same flow against the real authentik.

mod common;

use axum::{Router, http::StatusCode, routing::get};
use axum_login::login_required;
use common::{
    Browser,
    oidc::{FakeOidc, TestUser},
};
use scuba::{
    auth::{self, AuthSession, Backend},
    config::OidcConfig,
};
use sqlx::SqlitePool;

const CLIENT_ID: &str = "scuba-client";
const SECRET: &str = "a-long-enough-shared-secret-for-hs256";
const PUBLIC: &str = "http://scuba.test";

/// Everything one test needs: a fake IdP, the database, and a browser that talks to the app.
struct Fixture {
    idp: FakeOidc,
    db: SqlitePool,
    browser: Browser,
}

/// scuba's login routes plus a protected `/whoami` page (which shows the logged-in user's email),
/// wired like `main` does it. `secure` controls the session cookie's `Secure` flag.
async fn app_with(idp: &FakeOidc, db: SqlitePool, secure: bool) -> Router {
    let backend = Backend::new(
        db.clone(),
        &OidcConfig {
            issuer_url: idp.issuer(),
            client_id: CLIENT_ID.into(),
            client_secret: SECRET.into(),
        },
        PUBLIC,
    )
    .await
    .expect("discovery against fake idp");

    let protected = Router::new()
        .route(
            "/whoami",
            get(|auth: AuthSession| async move { auth.user.map(|u| u.email).unwrap_or_default() }),
        )
        .route_layer(login_required!(Backend, login_url = "/login"));
    let app = Router::new().merge(auth::router()).merge(protected);
    auth::with_auth_layers(app, db, backend, secure)
}

/// A fresh fake IdP, empty database, and a browser pointed at an app that trusts that IdP.
async fn fixture() -> Fixture {
    let idp = FakeOidc::start(CLIENT_ID, SECRET).await;
    let db = scuba::db::connect("sqlite::memory:").await.unwrap();
    let browser = Browser::new(app_with(&idp, db.clone(), false).await);
    Fixture { idp, db, browser }
}

/// Do the first half of a login: visit `login_uri`, follow it to the IdP, and return the callback
/// URL the IdP sends the user back to. The callback itself is *not* visited yet.
async fn start_login(fx: &mut Fixture, login_uri: &str) -> String {
    let r = fx.browser.get(login_uri).await;
    assert!(r.status.is_redirection(), "/login answered {}", r.status);
    fx.browser.idp_authorize(r.location()).await
}

/// A complete login. Returns scuba's response to the callback.
async fn login(fx: &mut Fixture) -> common::Resp {
    let cb = start_login(fx, "/login").await;
    fx.browser.get(&cb).await
}

/// Number of rows in `table` (trusted, hard-coded names only).
async fn count(db: &SqlitePool, table: &str) -> i64 {
    sqlx::query_as::<_, (i64,)>(sqlx::AssertSqlSafe(format!("select count(*) from {table}")))
        .fetch_one(db)
        .await
        .unwrap()
        .0
}

// ---- protection ----

#[tokio::test]
async fn protected_routes_redirect_anonymous_users_to_login_with_next() {
    let mut fx = fixture().await;
    let r = fx.browser.get("/whoami").await;
    assert!(r.status.is_redirection());
    assert_eq!(r.location(), "/login?next=%2Fwhoami");
}

#[tokio::test]
async fn public_routes_do_not_require_login() {
    let mut fx = fixture().await;
    let r = fx.browser.get("/logged-out").await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("/login"));
}

// ---- happy path ----

#[tokio::test]
async fn login_redirects_to_authentik_with_pkce_state_nonce_and_scopes() {
    let mut fx = fixture().await;
    let r = fx.browser.get("/login").await;
    assert!(
        r.location().starts_with(&fx.idp.issuer()),
        "{}",
        r.location()
    );
    fx.browser.idp_authorize(r.location()).await;

    let q = &fx.idp.authorize_queries()[0];
    assert_eq!(q["client_id"], CLIENT_ID);
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["redirect_uri"], format!("{PUBLIC}/auth/callback"));
    assert_eq!(q["code_challenge_method"], "S256");
    assert!(!q["code_challenge"].is_empty() && !q["state"].is_empty() && !q["nonce"].is_empty());
    let scopes: Vec<_> = q["scope"].split(' ').collect();
    for s in ["openid", "email", "profile"] {
        assert!(scopes.contains(&s), "missing scope {s}: {scopes:?}");
    }
}

#[tokio::test]
async fn full_login_creates_user_and_session() {
    let mut fx = fixture().await;
    let r = login(&mut fx).await;
    assert!(
        r.status.is_redirection(),
        "callback answered {} {}",
        r.status,
        r.body
    );
    assert_eq!(r.location(), "/");

    let who = fx.browser.get("/whoami").await;
    assert_eq!(
        (who.status, who.body.as_str()),
        (StatusCode::OK, "alice@example.com")
    );

    assert_eq!(count(&fx.db, "users").await, 1);
    let (sub, name): (String, String) = sqlx::query_as("select oidc_sub, display_name from users")
        .fetch_one(&fx.db)
        .await
        .unwrap();
    assert_eq!((sub.as_str(), name.as_str()), ("sub-alice", "alice"));
    assert!(
        count(&fx.db, "sessions").await >= 1,
        "session persisted in the db"
    );
}

#[tokio::test]
async fn code_exchange_sends_the_pkce_verifier() {
    let mut fx = fixture().await;
    login(&mut fx).await;
    let t = &fx.idp.token_requests()[0];
    assert_eq!(t["grant_type"], "authorization_code");
    assert!(t.get("code_verifier").is_some_and(|v| v.len() >= 43));
    assert_eq!(t["redirect_uri"], format!("{PUBLIC}/auth/callback"));
}

#[tokio::test]
async fn session_id_changes_on_login() {
    let mut fx = fixture().await;
    let cb = start_login(&mut fx, "/login").await;
    let before = fx
        .browser
        .cookies
        .get("id")
        .cloned()
        .expect("pre-login session cookie");
    fx.browser.get(&cb).await;
    let after = fx.browser.cookies.get("id").cloned().unwrap();
    assert_ne!(
        before, after,
        "session fixation: id must be rotated at login"
    );
}

#[tokio::test]
async fn second_login_with_same_sub_updates_profile_not_duplicates() {
    let mut fx = fixture().await;
    login(&mut fx).await;
    fx.browser.post("/logout").await;

    fx.idp.set_user(TestUser::new(
        "sub-alice",
        "alice@new.example.com",
        "alice-renamed",
    ));
    login(&mut fx).await;

    assert_eq!(count(&fx.db, "users").await, 1);
    let who = fx.browser.get("/whoami").await;
    assert_eq!(who.body, "alice@new.example.com");
    let (name,): (String,) = sqlx::query_as("select display_name from users")
        .fetch_one(&fx.db)
        .await
        .unwrap();
    assert_eq!(name, "alice-renamed");
}

#[tokio::test]
async fn different_subs_get_different_users() {
    let mut fx = fixture().await;
    login(&mut fx).await;
    fx.browser.post("/logout").await;
    fx.idp
        .set_user(TestUser::new("sub-bob", "bob@example.com", "bob"));
    login(&mut fx).await;
    assert_eq!(count(&fx.db, "users").await, 2);
    assert_eq!(fx.browser.get("/whoami").await.body, "bob@example.com");
}

#[tokio::test]
async fn visiting_login_while_authenticated_skips_the_idp() {
    let mut fx = fixture().await;
    login(&mut fx).await;
    let r = fx.browser.get("/login?next=/dns").await;
    assert_eq!(r.location(), "/dns");
    assert_eq!(
        fx.idp.authorize_queries().len(),
        1,
        "no second trip to authentik"
    );
}

// ---- next= handling ----

#[tokio::test]
async fn next_is_honoured_after_login() {
    let mut fx = fixture().await;
    let cb = start_login(&mut fx, "/login?next=/groups").await;
    let r = fx.browser.get(&cb).await;
    assert_eq!(r.location(), "/groups");
}

#[tokio::test]
async fn unsafe_next_values_fall_back_to_root() {
    for bad in ["//evil.com", "https://evil.com", "evil.com"] {
        let mut fx = fixture().await;
        let cb = start_login(&mut fx, &format!("/login?next={}", urlencode(bad))).await;
        let r = fx.browser.get(&cb).await;
        assert_eq!(r.location(), "/", "next={bad}");
    }
}

/// Percent-encode `s` the way it would appear in a query string.
fn urlencode(s: &str) -> String {
    reqwest::Url::parse_with_params("http://x/", [("n", s)])
        .unwrap()
        .query()
        .unwrap()
        .trim_start_matches("n=")
        .to_owned()
}

// ---- rejection paths ----

/// After a login that should have failed: still logged out, and no user was created.
async fn assert_anonymous(fx: &mut Fixture) {
    let r = fx.browser.get("/whoami").await;
    assert!(
        r.status.is_redirection(),
        "must still be anonymous, got {}",
        r.status
    );
    assert_eq!(
        count(&fx.db, "users").await,
        0,
        "no user may be created on failed login"
    );
}

#[tokio::test]
async fn state_mismatch_is_rejected() {
    let mut fx = fixture().await;
    let cb = start_login(&mut fx, "/login").await;
    let forged = cb.replace("state=", "state=forged");
    let r = fx.browser.get(&forged).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(
        fx.idp.token_requests().is_empty(),
        "must not exchange the code after a state mismatch"
    );
    assert_anonymous(&mut fx).await;
}

#[tokio::test]
async fn callback_without_a_started_login_is_rejected() {
    let mut fx = fixture().await;
    let r = fx.browser.get("/auth/callback?code=abc&state=xyz").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert!(fx.idp.token_requests().is_empty());
}

#[tokio::test]
async fn callback_is_single_use_even_if_the_idp_would_accept_the_code_again() {
    let mut fx = fixture().await;
    fx.idp.set_reusable_codes(true);
    let cb = start_login(&mut fx, "/login").await;
    assert!(fx.browser.get(&cb).await.status.is_redirection());
    assert_eq!(fx.idp.token_requests().len(), 1);

    let replay = fx.browser.get(&cb).await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        fx.idp.token_requests().len(),
        1,
        "replay must be stopped before reaching the token endpoint"
    );
}

#[tokio::test]
async fn replayed_callback_cannot_log_in_a_fresh_browser() {
    let mut fx = fixture().await;
    fx.idp.set_reusable_codes(true);
    let cb = start_login(&mut fx, "/login").await;
    fx.browser.get(&cb).await;

    // an attacker's browser has no pending login of its own
    let mut attacker = Browser::new(app_with(&fx.idp, fx.db.clone(), false).await);
    assert_eq!(attacker.get(&cb).await.status, StatusCode::UNAUTHORIZED);
    assert!(attacker.get("/whoami").await.status.is_redirection());
}

#[tokio::test]
async fn callback_is_single_use() {
    let mut fx = fixture().await;
    let cb = start_login(&mut fx, "/login").await;
    assert!(fx.browser.get(&cb).await.status.is_redirection());
    fx.browser.post("/logout").await;

    let replay = fx.browser.get(&cb).await;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
    let r = fx.browser.get("/whoami").await;
    assert!(r.status.is_redirection(), "replay must not log in");
}

#[tokio::test]
async fn idp_error_response_is_rejected() {
    let mut fx = fixture().await;
    fx.browser.get("/login").await;
    let r = fx
        .browser
        .get("/auth/callback?error=access_denied&state=whatever")
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_anonymous(&mut fx).await;
}

#[tokio::test]
async fn missing_code_or_state_is_rejected() {
    let mut fx = fixture().await;
    fx.browser.get("/login").await;
    assert_eq!(
        fx.browser.get("/auth/callback").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn id_token_with_wrong_nonce_is_rejected() {
    let mut fx = fixture().await;
    fx.idp.set_tamper_nonce(true);
    let r = login(&mut fx).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_anonymous(&mut fx).await;
}

#[tokio::test]
async fn id_token_signed_with_a_different_secret_is_rejected() {
    let idp = FakeOidc::start(CLIENT_ID, "some-other-secret-than-the-one-we-have").await;
    let db = scuba::db::connect("sqlite::memory:").await.unwrap();
    let mut fx = Fixture {
        browser: Browser::new(app_with(&idp, db.clone(), false).await),
        idp,
        db,
    };
    let r = login(&mut fx).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_anonymous(&mut fx).await;
}

#[tokio::test]
async fn missing_email_claim_is_rejected() {
    let mut fx = fixture().await;
    fx.idp.set_user(TestUser {
        sub: "sub-x".into(),
        email: None,
        username: "x".into(),
    });
    let r = login(&mut fx).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_anonymous(&mut fx).await;
}

#[tokio::test]
async fn failed_login_page_offers_a_retry_link() {
    let mut fx = fixture().await;
    let r = fx.browser.get("/auth/callback?code=a&state=b").await;
    assert!(r.body.contains("href=\"/login\""));
}

// ---- logout & session lifetime ----

#[tokio::test]
async fn logout_ends_the_session() {
    let mut fx = fixture().await;
    login(&mut fx).await;
    let r = fx.browser.post("/logout").await;
    assert_eq!(r.location(), "/logged-out");
    let after = fx.browser.get("/whoami").await;
    assert!(after.status.is_redirection());
}

#[tokio::test]
async fn logout_requires_post() {
    let mut fx = fixture().await;
    login(&mut fx).await;
    assert_eq!(
        fx.browser.get("/logout").await.status,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        fx.browser.get("/whoami").await.status,
        StatusCode::OK,
        "still logged in"
    );
}

#[tokio::test]
async fn old_session_cookie_is_useless_after_logout() {
    let mut fx = fixture().await;
    login(&mut fx).await;
    let stolen = fx.browser.cookies.clone();
    fx.browser.post("/logout").await;
    fx.browser.cookies = stolen;
    assert!(fx.browser.get("/whoami").await.status.is_redirection());
}

#[tokio::test]
async fn deleted_user_is_logged_out_on_next_request() {
    let mut fx = fixture().await;
    login(&mut fx).await;
    sqlx::query("delete from users")
        .execute(&fx.db)
        .await
        .unwrap();
    assert!(fx.browser.get("/whoami").await.status.is_redirection());
}

#[tokio::test]
async fn expired_session_row_logs_the_user_out() {
    let mut fx = fixture().await;
    login(&mut fx).await;
    sqlx::query("update sessions set expiry = 0")
        .execute(&fx.db)
        .await
        .unwrap();
    assert!(fx.browser.get("/whoami").await.status.is_redirection());
}

#[tokio::test]
async fn garbage_session_cookie_is_treated_as_anonymous() {
    let mut fx = fixture().await;
    fx.browser
        .cookies
        .insert("id".into(), "not-a-real-session".into());
    assert!(fx.browser.get("/whoami").await.status.is_redirection());
}

// ---- cookie attributes ----

#[tokio::test]
async fn session_cookie_is_httponly_lax_and_not_secure_over_http() {
    let mut fx = fixture().await;
    let r = fx.browser.get("/login").await;
    let c = r.set_cookies().join("\n").to_ascii_lowercase();
    assert!(c.contains("httponly"), "{c}");
    assert!(c.contains("samesite=lax"), "{c}");
    assert!(!c.contains("secure"), "{c}");
}

#[tokio::test]
async fn session_cookie_is_secure_when_configured() {
    let idp = FakeOidc::start(CLIENT_ID, SECRET).await;
    let db = scuba::db::connect("sqlite::memory:").await.unwrap();
    let mut b = Browser::new(app_with(&idp, db, true).await);
    let c = b
        .get("/login")
        .await
        .set_cookies()
        .join("\n")
        .to_ascii_lowercase();
    assert!(c.contains("secure"), "{c}");
}

// ---- startup ----

#[tokio::test]
async fn backend_new_fails_when_discovery_fails() {
    let db = scuba::db::connect("sqlite::memory:").await.unwrap();
    let r = Backend::new(
        db,
        &OidcConfig {
            issuer_url: "http://127.0.0.1:1/application/o/x/".into(),
            client_id: "a".into(),
            client_secret: "b".into(),
        },
        PUBLIC,
    )
    .await;
    assert!(r.is_err());
}

#[tokio::test]
async fn backend_new_rejects_an_issuer_that_does_not_match_the_metadata() {
    let idp = FakeOidc::start(CLIENT_ID, SECRET).await;
    let db = scuba::db::connect("sqlite::memory:").await.unwrap();
    // missing trailing slash => the discovery document's `issuer` no longer matches
    let wrong = idp.issuer().trim_end_matches('/').to_owned();
    let r = Backend::new(
        db,
        &OidcConfig {
            issuer_url: wrong,
            client_id: CLIENT_ID.into(),
            client_secret: SECRET.into(),
        },
        PUBLIC,
    )
    .await;
    assert!(r.is_err());
}
