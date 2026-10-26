//! Helpers shared by the integration tests.
//!
//! * [`netbird::FakeNetbird`] and [`oidc::FakeOidc`] are in-process fakes of the two services
//!   scuba talks to.
//! * [`Browser`] drives the app's router in-process with cookies, like a minimal web browser.

// Each test file uses a different subset of these helpers.
#![allow(dead_code)]

pub mod netbird;
pub mod oidc;

use std::collections::HashMap;

use axum::{
    Router,
    body::Body,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use tower::ServiceExt;

/// What came back from [`Browser::request`], with the body already read.
pub struct Resp {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: String,
}

impl Resp {
    /// The `Location` header of a redirect.
    pub fn location(&self) -> &str {
        self.headers
            .get(header::LOCATION)
            .expect("no Location header")
            .to_str()
            .unwrap()
    }

    /// Every `Set-Cookie` header, verbatim.
    pub fn set_cookies(&self) -> Vec<String> {
        self.headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect()
    }
}

/// A minimal web browser: it keeps cookies between requests to the app (served in-process, no
/// network), and can visit the fake identity provider over real HTTP.
/// It never follows redirects by itself, so tests see every step.
pub struct Browser {
    app: Router,
    /// The cookie jar, by cookie name. Tests may read or replace it (e.g. to replay a session).
    pub cookies: HashMap<String, String>,
    /// For talking to the identity provider; doesn't follow redirects either.
    http: reqwest::Client,
}

impl Browser {
    pub fn new(app: Router) -> Self {
        Self {
            app,
            cookies: Default::default(),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap(),
        }
    }

    /// Send a request to the app with the stored cookies, and remember any cookies it sets.
    pub async fn request(&mut self, method: Method, uri: &str) -> Resp {
        let mut request = Request::builder().method(method).uri(uri);
        if !self.cookies.is_empty() {
            let cookie_header: Vec<_> = self
                .cookies
                .iter()
                .map(|(name, value)| format!("{name}={value}"))
                .collect();
            request = request.header(header::COOKIE, cookie_header.join("; "));
        }

        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        let (parts, body) = response.into_parts();
        let body_bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();

        let response = Resp {
            status: parts.status,
            headers: parts.headers,
            body: String::from_utf8_lossy(&body_bytes).into_owned(),
        };
        self.remember_cookies(&response);
        response
    }

    pub async fn get(&mut self, uri: &str) -> Resp {
        self.request(Method::GET, uri).await
    }

    pub async fn post(&mut self, uri: &str) -> Resp {
        self.request(Method::POST, uri).await
    }

    /// Apply the `Set-Cookie` headers of a response: store new values, drop expired cookies.
    fn remember_cookies(&mut self, response: &Resp) {
        for set_cookie in response.set_cookies() {
            let name_and_value = set_cookie.split(';').next().unwrap();
            let (name, value) = name_and_value.split_once('=').unwrap();
            let expired = value.is_empty() || set_cookie.to_ascii_lowercase().contains("max-age=0");
            if expired {
                self.cookies.remove(name);
            } else {
                self.cookies.insert(name.into(), value.into());
            }
        }
    }

    /// Do what the user's browser does after `/login` redirects to the identity provider: visit
    /// its authorize URL. Returns where it sends the user back to, as a path and query that can
    /// be passed to [`Browser::get`].
    pub async fn idp_authorize(&self, authorize_url: &str) -> String {
        let response = self.http.get(authorize_url).send().await.unwrap();
        assert!(
            response.status().is_redirection(),
            "the identity provider answered {}",
            response.status()
        );
        let location = response.headers()[header::LOCATION].to_str().unwrap();
        let callback = reqwest::Url::parse(location).unwrap();
        format!(
            "{}?{}",
            callback.path(),
            callback.query().unwrap_or_default()
        )
    }
}
