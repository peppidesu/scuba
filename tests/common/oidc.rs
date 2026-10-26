//! An in-process fake of authentik, enough for scuba's login flow: discovery, `authorize`
//! and `token`, served on a random local port.
//!
//! It is strict where a real provider is strict: authorization codes are single-use, the PKCE
//! verifier must match the challenge sent to `authorize`, and the token request must repeat the
//! redirect URI. ID tokens are signed with HS256 using the client secret, which openidconnect's
//! confidential-client verifier accepts, so no key pair is needed.
//!
//! It can also misbehave on request (wrong nonce, reusable codes, a user with no email), to test
//! how scuba reacts.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Form, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use openidconnect::{
    Audience, EmptyAdditionalClaims, EndUserEmail, EndUserUsername, IssuerUrl, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, StandardClaims, SubjectIdentifier,
    core::{CoreGenderClaim, CoreHmacKey, CoreIdToken, CoreIdTokenClaims, CoreJwsSigningAlgorithm},
};
use serde_json::json;

/// authentik serves each application under its own path; the fake mimics that.
const APP_PATH: &str = "/application/o/scuba";

/// Whoever "logs in" at the fake IdP next.
#[derive(Clone)]
pub struct TestUser {
    /// The stable OIDC `sub`.
    pub sub: String,
    /// `None` leaves the `email` claim out entirely.
    pub email: Option<String>,
    pub username: String,
}

impl TestUser {
    pub fn new(sub: &str, email: &str, username: &str) -> Self {
        Self {
            sub: sub.into(),
            email: Some(email.into()),
            username: username.into(),
        }
    }
}

/// An authorization code that has been issued but not yet exchanged.
#[derive(Clone)]
struct IssuedCode {
    nonce: String,
    pkce_challenge: String,
    redirect_uri: String,
}

struct Provider {
    /// `http://127.0.0.1:<port>`
    base: String,
    client_id: String,
    client_secret: String,
    user: TestUser,

    // misbehaviour switches
    tamper_nonce: bool,
    reusable_codes: bool,

    codes: HashMap<String, IssuedCode>,
    next_code: u32,

    // what scuba sent us, for assertions
    authorize_queries: Vec<HashMap<String, String>>,
    token_requests: Vec<HashMap<String, String>>,
}

type Shared = Arc<Mutex<Provider>>;

pub struct FakeOidc {
    pub base: String,
    provider: Shared,
}

impl FakeOidc {
    pub async fn start(client_id: &str, client_secret: &str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let provider: Shared = Arc::new(Mutex::new(Provider {
            base: base.clone(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            user: TestUser::new("sub-alice", "alice@example.com", "alice"),
            tamper_nonce: false,
            reusable_codes: false,
            codes: Default::default(),
            next_code: 0,
            authorize_queries: vec![],
            token_requests: vec![],
        }));

        let app = Router::new()
            .route(
                &format!("{APP_PATH}/.well-known/openid-configuration"),
                get(discovery),
            )
            .route(
                &format!("{APP_PATH}/jwks/"),
                get(|| async { Json(json!({"keys": []})) }), // HS256 needs no public keys
            )
            .route(&format!("{APP_PATH}/authorize/"), get(authorize))
            .route(&format!("{APP_PATH}/token/"), post(token))
            .with_state(provider.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        Self { base, provider }
    }

    /// The value for `OIDC_ISSUER_URL` (with the trailing slash, like authentik's).
    pub fn issuer(&self) -> String {
        format!("{}{APP_PATH}/", self.base)
    }

    // ---- steer the fake ----

    /// Who logs in from now on.
    pub fn set_user(&self, user: TestUser) {
        self.provider.lock().unwrap().user = user;
    }

    /// Put the wrong nonce in ID tokens: scuba must reject them.
    pub fn set_tamper_nonce(&self, on: bool) {
        self.provider.lock().unwrap().tamper_nonce = on;
    }

    /// Accept an authorization code more than once, to prove scuba doesn't rely on the IdP to
    /// stop replays.
    pub fn set_reusable_codes(&self, on: bool) {
        self.provider.lock().unwrap().reusable_codes = on;
    }

    // ---- inspect what scuba sent ----

    pub fn authorize_queries(&self) -> Vec<HashMap<String, String>> {
        self.provider.lock().unwrap().authorize_queries.clone()
    }

    pub fn token_requests(&self) -> Vec<HashMap<String, String>> {
        self.provider.lock().unwrap().token_requests.clone()
    }
}

// ---- endpoints ----------------------------------------------------------------------------

async fn discovery(State(provider): State<Shared>) -> Json<serde_json::Value> {
    let base = provider.lock().unwrap().base.clone();
    Json(json!({
        "issuer": format!("{base}{APP_PATH}/"),
        "authorization_endpoint": format!("{base}{APP_PATH}/authorize/"),
        "token_endpoint": format!("{base}{APP_PATH}/token/"),
        "jwks_uri": format!("{base}{APP_PATH}/jwks/"),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["HS256"],
    }))
}

/// Pretend the user logged in and approved: issue a code and redirect straight back.
async fn authorize(
    State(provider): State<Shared>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let mut provider = provider.lock().unwrap();
    provider.authorize_queries.push(query.clone());

    let (Some(redirect_uri), Some(state)) = (query.get("redirect_uri"), query.get("state")) else {
        return (StatusCode::BAD_REQUEST, "missing redirect_uri/state").into_response();
    };
    let right_client = query.get("client_id") == Some(&provider.client_id);
    let right_flow = query.get("response_type").map(String::as_str) == Some("code");
    if !right_client || !right_flow {
        return (StatusCode::BAD_REQUEST, "bad client or response_type").into_response();
    }

    provider.next_code += 1;
    let code = format!("code-{}", provider.next_code);
    provider.codes.insert(
        code.clone(),
        IssuedCode {
            nonce: query.get("nonce").cloned().unwrap_or_default(),
            pkce_challenge: query.get("code_challenge").cloned().unwrap_or_default(),
            redirect_uri: redirect_uri.clone(),
        },
    );

    let mut callback = reqwest::Url::parse(redirect_uri).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("code", &code)
        .append_pair("state", state);
    Redirect::to(callback.as_str()).into_response()
}

fn oauth_error(error: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({"error": error}))).into_response()
}

/// Exchange a code for tokens, if the request is as strict as a real provider would demand.
async fn token(
    State(provider): State<Shared>,
    Form(request): Form<HashMap<String, String>>,
) -> Response {
    let mut provider = provider.lock().unwrap();
    provider.token_requests.push(request.clone());

    if request.get("grant_type").map(String::as_str) != Some("authorization_code") {
        return oauth_error("unsupported_grant_type");
    }

    // The code must exist, and is used up by this exchange (unless told otherwise).
    let reusable = provider.reusable_codes;
    let issued = request.get("code").and_then(|code| {
        if reusable {
            provider.codes.get(code).cloned()
        } else {
            provider.codes.remove(code)
        }
    });
    let Some(issued) = issued else {
        return oauth_error("invalid_grant");
    };

    // The redirect URI must be the one the code was issued for.
    if request.get("redirect_uri") != Some(&issued.redirect_uri) {
        return oauth_error("invalid_grant");
    }

    // PKCE (S256): the verifier must hash to the challenge sent to `authorize`.
    let Some(verifier) = request.get("code_verifier") else {
        return oauth_error("invalid_grant");
    };
    let expected =
        PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(verifier.clone()));
    if expected.as_str() != issued.pkce_challenge {
        return oauth_error("invalid_grant");
    }

    let nonce = if provider.tamper_nonce {
        "not-the-nonce".to_owned()
    } else {
        issued.nonce
    };
    let id_token = signed_id_token(&provider, nonce);

    Json(json!({
        "access_token": "at-123",
        "token_type": "Bearer",
        "expires_in": 300,
        "id_token": id_token,
    }))
    .into_response()
}

/// An ID token for the current test user, valid for five minutes, signed with the client secret.
fn signed_id_token(provider: &Provider, nonce: String) -> CoreIdToken {
    let mut user_claims =
        StandardClaims::<CoreGenderClaim>::new(SubjectIdentifier::new(provider.user.sub.clone()))
            .set_preferred_username(Some(EndUserUsername::new(provider.user.username.clone())));
    if let Some(email) = &provider.user.email {
        user_claims = user_claims.set_email(Some(EndUserEmail::new(email.clone())));
    }

    let claims = CoreIdTokenClaims::new(
        IssuerUrl::new(format!("{}{APP_PATH}/", provider.base)).unwrap(),
        vec![Audience::new(provider.client_id.clone())],
        Utc::now() + Duration::minutes(5),
        Utc::now(),
        user_claims,
        EmptyAdditionalClaims {},
    )
    .set_nonce(Some(Nonce::new(nonce)));

    CoreIdToken::new(
        claims,
        &CoreHmacKey::new(provider.client_secret.as_bytes().to_vec()),
        CoreJwsSigningAlgorithm::HmacSha256,
        None,
        None,
    )
    .unwrap()
}
