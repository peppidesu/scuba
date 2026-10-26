//! Login against authentik: OIDC authorization-code flow with PKCE, sessions via
//! axum-login and tower-sessions.
//!
//! The flow, as seen by the user:
//!
//! 1. `GET /login` stores a fresh `{state, nonce, PKCE verifier, next}` in the session
//!    (see [`Pending`]) and redirects to authentik.
//! 2. The user logs in at authentik, which redirects back to
//! 3. `GET /auth/callback?code=…&state=…`: we check `state` against the session, exchange the
//!    code for tokens, verify the ID token ([`Backend::authenticate`]), then log the user in
//!    and redirect to `next`.
//! 4. `POST /logout` ends our session. authentik's own SSO session is left alone.
//!
//! Who is *allowed* to log in is decided in authentik (application policy bindings), not here.
//! The redirect URI to register in authentik is `{SCUBA_PUBLIC_URL}/auth/callback`.
//!
//! Module layout: this file has the backend, the handlers and the layer setup; [`sessions`]
//! adapts our database to tower-sessions.

use axum::{
    Router,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use axum_login::{AuthManagerLayerBuilder, AuthUser, AuthnBackend, UserId};
use maud::html;
use openidconnect::{
    AccessTokenHash, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet,
    EndpointNotSet, EndpointSet, IssuerUrl, Nonce, OAuth2TokenResponse, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
    core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata},
    reqwest as oidc_http,
};
use serde::{Deserialize, Serialize};
use tower_sessions::{
    Expiry, Session, SessionManagerLayer,
    cookie::{SameSite, time::Duration},
};

use crate::{components, config::OidcConfig, db, db::users::User};

pub mod sessions;

/// Session key under which a login in progress is stored.
const PENDING_KEY: &str = "oidc_pending";

/// The request extractor handlers use to see who is logged in.
pub type AuthSession = axum_login::AuthSession<Backend>;

// ---- who is logged in -------------------------------------------------------------------

impl AuthUser for User {
    type Id = i64;

    fn id(&self) -> i64 {
        self.id
    }

    /// axum-login stores this in the session and ends the session if it ever changes.
    /// The OIDC `sub` never changes for a given person, so this only ever ends a session
    /// if the row is replaced by a different identity.
    fn session_auth_hash(&self) -> &[u8] {
        self.oidc_sub.as_bytes()
    }
}

// ---- OIDC client and backend ----------------------------------------------------------------

/// The authentik client as returned by discovery: the authorize endpoint is known for sure,
/// while the token and userinfo endpoints are "maybe set" (they come from the provider metadata).
type OidcClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

struct Oidc {
    client: OidcClient,
    http: oidc_http::Client,
}

/// axum-login backend: turns an OIDC callback into a [`User`] row and loads users from the
/// session. Cheap to clone.
#[derive(Clone)]
pub struct Backend {
    db: sqlx::SqlitePool,
    oidc: std::sync::Arc<Oidc>,
}

impl Backend {
    /// Performs OIDC discovery against `cfg.issuer_url`, so authentik must be reachable.
    /// For authentik the issuer is `https://<host>/application/o/<slug>/` — the trailing
    /// slash matters, the ID token's `iss` has to match it exactly.
    pub async fn new(
        db: sqlx::SqlitePool,
        cfg: &OidcConfig,
        public_url: &str,
    ) -> anyhow::Result<Self> {
        let http = oidc_http::ClientBuilder::new()
            // following redirects would open the client to SSRF
            .redirect(oidc_http::redirect::Policy::none())
            .build()?;

        let issuer = IssuerUrl::new(cfg.issuer_url.clone())?;
        let metadata = CoreProviderMetadata::discover_async(issuer, &http).await?;

        let redirect_uri = format!("{}/auth/callback", public_url.trim_end_matches('/'));
        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(cfg.client_id.clone()),
            Some(ClientSecret::new(cfg.client_secret.clone())),
        )
        .set_redirect_uri(RedirectUrl::new(redirect_uri)?);

        Ok(Self {
            db,
            oidc: std::sync::Arc::new(Oidc { client, http }),
        })
    }
}

/// What the callback handler hands to [`Backend::authenticate`]: the authorization code from
/// authentik plus the secrets we stored in the session when the login started.
pub struct OidcCallback {
    pub code: String,
    pub pkce_verifier: String,
    pub nonce: String,
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    /// authentik said no, or sent something we can't trust.
    #[error("oidc: {0}")]
    Oidc(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

fn oidc_err(e: impl std::error::Error) -> LoginError {
    LoginError::Oidc(e.to_string())
}

impl AuthnBackend for Backend {
    type User = User;
    type Credentials = OidcCallback;
    type Error = LoginError;

    /// Exchange the authorization code, verify the ID token, and find-or-create the user.
    async fn authenticate(&self, callback: OidcCallback) -> Result<Option<User>, LoginError> {
        let client = &self.oidc.client;

        // 1. Trade the code (plus our PKCE verifier) for tokens.
        let token = client
            .exchange_code(AuthorizationCode::new(callback.code))
            .map_err(oidc_err)?
            .set_pkce_verifier(PkceCodeVerifier::new(callback.pkce_verifier))
            .request_async(&self.oidc.http)
            .await
            .map_err(oidc_err)?;

        // 2. Verify the ID token: signature, issuer, audience, expiry and our nonce.
        let id_token = token
            .id_token()
            .ok_or_else(|| LoginError::Oidc("no id_token returned".into()))?;
        let verifier = client.id_token_verifier();
        let claims = id_token
            .claims(&verifier, &Nonce::new(callback.nonce))
            .map_err(oidc_err)?;

        // 3. If the ID token vouches for the access token (`at_hash`), check that it does.
        if let Some(expected) = claims.access_token_hash() {
            let actual = AccessTokenHash::from_token(
                token.access_token(),
                id_token.signing_alg().map_err(oidc_err)?,
                id_token.signing_key(&verifier).map_err(oidc_err)?,
            )
            .map_err(oidc_err)?;
            if actual != *expected {
                return Err(LoginError::Oidc("access token hash mismatch".into()));
            }
        }

        // 4. Read who this is. Email is required (it links the person to their NetBird user).
        let email = claims
            .email()
            .ok_or_else(|| {
                LoginError::Oidc(
                    "no email claim (is the `email` scope mapped in authentik?)".into(),
                )
            })?
            .as_str();
        let display_name = claims
            .preferred_username()
            .map(|name| name.as_str())
            .or_else(|| {
                claims
                    .name()
                    .and_then(|name| name.get(None))
                    .map(|name| name.as_str())
            })
            .unwrap_or(email);

        // 5. Keyed on the stable `sub`; email and name are refreshed on every login.
        let user =
            db::users::upsert_by_sub(&self.db, claims.subject().as_str(), email, display_name)
                .await?;
        Ok(Some(user))
    }

    async fn get_user(&self, id: &UserId<Self>) -> Result<Option<User>, LoginError> {
        Ok(db::users::by_id(&self.db, *id).await?)
    }
}

// ---- routes -------------------------------------------------------------------------------

/// Login, callback and logout. These are public: they are not behind `login_required!`.
pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/login", get(login))
        .route("/auth/callback", get(callback))
        .route("/logout", post(logout))
        .route("/logged-out", get(logged_out))
}

/// A login in progress. Written by `/login`, consumed (exactly once) by `/auth/callback`.
#[derive(Serialize, Deserialize)]
struct Pending {
    /// Random value echoed back by authentik; proves the callback belongs to *this* browser.
    state: String,
    /// Must reappear inside the ID token; ties the token to this login attempt.
    nonce: String,
    /// The secret half of the PKCE challenge sent to authentik.
    pkce_verifier: String,
    /// Where to send the user afterwards; already checked by [`safe_next`].
    next: Option<String>,
}

/// Only local absolute paths are allowed as a post-login destination, so that
/// `/login?next=https://evil.example` can't be used as an open redirect.
fn safe_next(next: Option<String>) -> Option<String> {
    next.filter(|n| n.starts_with('/') && !n.starts_with("//") && !n.contains('\\'))
}

#[derive(Deserialize)]
struct LoginQuery {
    next: Option<String>,
}

/// Start a login: remember what we'll need to verify the answer, then send the user to authentik.
async fn login(
    auth: AuthSession,
    session: Session,
    Query(query): Query<LoginQuery>,
) -> Result<Redirect, AuthError> {
    let next = safe_next(query.next);
    if auth.user.is_some() {
        return Ok(Redirect::to(next.as_deref().unwrap_or("/")));
    }

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (authorize_url, state, nonce) = auth
        .backend
        .oidc
        .client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("email".into()))
        .add_scope(Scope::new("profile".into()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    session
        .insert(
            PENDING_KEY,
            Pending {
                state: state.secret().clone(),
                nonce: nonce.secret().clone(),
                pkce_verifier: pkce_verifier.secret().clone(),
                next,
            },
        )
        .await?;
    Ok(Redirect::to(authorize_url.as_str()))
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    /// Set by authentik instead of `code` when something went wrong.
    error: Option<String>,
}

/// Finish a login: check the answer against what `/login` stored, then log the user in.
async fn callback(
    mut auth: AuthSession,
    session: Session,
    Query(query): Query<CallbackQuery>,
) -> Result<Redirect, AuthError> {
    // Single use: `remove` takes it out of the session even if a later check fails,
    // so a callback URL can never be replayed.
    let pending: Pending = session
        .remove(PENDING_KEY)
        .await?
        .ok_or(AuthError::Denied("no pending login in session"))?;

    if query.error.is_some() {
        return Err(AuthError::Denied("authentik returned an error"));
    }
    let (Some(code), Some(state)) = (query.code, query.state) else {
        return Err(AuthError::Denied("missing code/state"));
    };
    if state != pending.state {
        return Err(AuthError::Denied("state mismatch"));
    }

    let user = auth
        .authenticate(OidcCallback {
            code,
            pkce_verifier: pending.pkce_verifier,
            nonce: pending.nonce,
        })
        .await?
        .ok_or(AuthError::Denied("authentication rejected"))?;

    // axum-login also rotates the session id here, which prevents session fixation.
    auth.login(&user).await?;

    Ok(Redirect::to(pending.next.as_deref().unwrap_or("/")))
}

async fn logout(mut auth: AuthSession) -> Result<Redirect, AuthError> {
    auth.logout().await?;
    Ok(Redirect::to("/logged-out"))
}

/// Not a redirect to `/login`: authentik's SSO session would just log the user straight back in.
async fn logged_out() -> impl IntoResponse {
    components::page(
        "Logged out",
        html! { p { "You have been logged out. " a href="/login" { "Log in again" } } },
    )
}

// ---- errors -------------------------------------------------------------------------------

/// Everything that can go wrong in the handlers above. Details are logged, never shown:
/// the user only ever sees "Login failed." or "Something went wrong.".
enum AuthError {
    Session(tower_sessions::session::Error),
    Login(LoginError),
    /// A check failed; the string says which (for the log).
    Denied(&'static str),
}

impl From<tower_sessions::session::Error> for AuthError {
    fn from(e: tower_sessions::session::Error) -> Self {
        AuthError::Session(e)
    }
}

impl From<axum_login::Error<Backend>> for AuthError {
    fn from(e: axum_login::Error<Backend>) -> Self {
        match e {
            axum_login::Error::Backend(e) => AuthError::Login(e),
            axum_login::Error::Session(e) => AuthError::Session(e),
        }
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AuthError::Denied(reason) => {
                tracing::warn!("login rejected: {reason}");
                (StatusCode::UNAUTHORIZED, "Login failed.")
            }
            AuthError::Login(e) => {
                tracing::warn!("login failed: {e}");
                (StatusCode::UNAUTHORIZED, "Login failed.")
            }
            AuthError::Session(e) => {
                tracing::error!("session error: {e}");
                (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong.")
            }
        };
        let page = components::page(
            "Login",
            html! { p { (message) " " a href="/login" { "Try again" } } },
        );
        (status, page).into_response()
    }
}

// ---- layers -------------------------------------------------------------------------------

/// Adds the session and auth layers to `router`; shared by `main` and the tests.
///
/// The session cookie is `SameSite=Lax`: it survives the top-level redirect back from
/// authentik, but isn't sent on cross-site POSTs. `secure` should be true whenever the site
/// is served over https. Sessions end after 7 days without activity.
pub fn with_auth_layers(
    router: Router,
    db: sqlx::SqlitePool,
    backend: Backend,
    secure: bool,
) -> Router {
    let sessions = SessionManagerLayer::new(sessions::PoolSessionStore(db))
        .with_secure(secure)
        .with_same_site(SameSite::Lax)
        .with_expiry(Expiry::OnInactivity(Duration::days(7)));
    router.layer(AuthManagerLayerBuilder::new(backend, sessions).build())
}

#[cfg(test)]
mod tests {
    use super::safe_next;

    /// `safe_next` on a plain string, for readable assertions.
    fn checked(s: &str) -> Option<String> {
        safe_next(Some(s.to_owned()))
    }

    #[test]
    fn accepts_local_absolute_paths() {
        assert_eq!(checked("/"), Some("/".into()));
        assert_eq!(checked("/dns"), Some("/dns".into()));
        assert_eq!(
            checked("/groups/3/access?x=1"),
            Some("/groups/3/access?x=1".into())
        );
    }

    #[test]
    fn rejects_anything_that_could_leave_the_site() {
        for bad in [
            "//evil.com",
            "///evil.com",
            "https://evil.com",
            "http://evil.com/x",
            "evil.com",
            "",
            "/\\evil.com",
            "\\\\evil.com",
        ] {
            assert_eq!(checked(bad), None, "{bad:?} must be rejected");
        }
    }

    #[test]
    fn none_stays_none() {
        assert_eq!(safe_next(None), None);
    }
}
