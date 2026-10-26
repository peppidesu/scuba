//! Checks against the REAL dev stack (`dev/compose.yaml`), not the in-process fakes.
//!
//! The other integration tests use fakes, which are only as accurate as our guesses about
//! NetBird and authentik. These tests are the reality check, and they have caught real
//! differences before (NetBird sends `null` for empty lists, answers 404 for a name filter
//! with no match; authentik needs explicit grant types). Run them with:
//!
//! ```sh
//! docker compose -f dev/compose.yaml up -d && dev/setup.sh
//! cargo test --test dev_stack -- --ignored --test-threads=1
//! ```
//!
//! They are `#[ignore]`d so a plain `cargo test` needs no Docker.

mod common;

use std::collections::HashMap;

use axum::{Router, routing::get};
use axum_login::login_required;
use common::Browser;
use scuba::{
    auth::{self, AuthSession, Backend},
    config::OidcConfig,
    netbird::{self, NetbirdClient},
};
use serde_json::{Value, json};

// ---- setup helpers ------------------------------------------------------------------------

/// The settings `dev/setup.sh` wrote to `dev/scuba.env`.
fn dev_env() -> HashMap<String, String> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/dev/scuba.env");
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("{path} is missing: run dev/setup.sh first"));
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

/// A client for the dev NetBird, using the token from `dev/scuba.env`.
fn netbird_client() -> NetbirdClient {
    let env = dev_env();
    NetbirdClient::new(
        env["NETBIRD_API_URL"].clone(),
        env["NETBIRD_API_TOKEN"].clone(),
    )
    .unwrap()
}

/// `prefix` plus a few digits derived from the clock, so reruns don't collide with leftovers.
fn unique(prefix: &str) -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("{prefix}{}", millis % 1_000_000_000)
}

// ---- NetBird ------------------------------------------------------------------------------

/// One pass through everything the client does, in the order a real session would.
#[tokio::test]
#[ignore = "needs the dev stack"]
async fn netbird_client_works_against_the_real_api() {
    let nb = netbird_client();
    nb.list_peers().await.expect("list_peers");

    // The built-in `All` group exists. Empty, it has `peers: null` — a shape we must parse.
    let all = nb
        .find_group("All")
        .await
        .unwrap()
        .expect("built-in All group");
    assert_eq!(all.issued.as_deref(), Some("api"));

    // groups
    let group_name = unique("scuba-test-");
    let group = nb.create_group(&group_name).await.expect("create_group");
    assert_eq!(nb.get_group(&group.id).await.unwrap().peers.len(), 0);
    assert_eq!(
        nb.find_group(&group_name).await.unwrap().unwrap().id,
        group.id
    );

    // a one-way policy from All to the new group
    let policy_id = nb
        .create_policy(&group_name, &all.id, &group.id)
        .await
        .expect("create_policy");
    nb.delete_policy(&policy_id).await.expect("delete_policy");

    // dns: a zone, a record in it; update the record; delete everything again
    let domain = unique("scubatest");
    let zone = nb
        .create_zone(&domain, std::slice::from_ref(&all.id))
        .await
        .expect("create_zone");
    assert_eq!(nb.find_zone(&domain).await.unwrap().unwrap().id, zone.id);

    let fqdn = format!("web.{domain}");
    let record = nb
        .create_record(&zone.id, &fqdn, "A", "100.64.0.9", 60)
        .await
        .expect("create_record");
    assert_eq!(nb.list_records(&zone.id).await.unwrap().len(), 1);

    let updated = nb
        .update_record(&zone.id, &record.id, &fqdn, "A", "100.64.0.10", 120)
        .await
        .expect("update_record");
    assert_eq!(
        (updated.content.as_str(), updated.ttl),
        ("100.64.0.10", 120)
    );

    nb.delete_record(&zone.id, &record.id)
        .await
        .expect("delete_record");
    nb.delete_zone(&zone.id).await.expect("delete_zone");
    assert!(nb.find_zone(&domain).await.unwrap().is_none());

    // The name filter finds nothing (a 404 from NetBird) once the group is gone.
    nb.delete_group(&group.id).await.expect("delete_group");
    assert!(nb.find_group(&group_name).await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "needs the dev stack"]
async fn real_netbird_rejects_a_bad_token_and_reports_missing_things() {
    let env = dev_env();
    let bad_token = NetbirdClient::new(
        env["NETBIRD_API_URL"].clone(),
        "nbp_not-a-real-token".into(),
    )
    .unwrap();
    assert!(matches!(
        bad_token.list_peers().await,
        Err(netbird::Error::Api { status: 401, .. })
    ));

    assert!(matches!(
        netbird_client().get_peer("does-not-exist").await,
        Err(netbird::Error::NotFound)
    ));
}

// ---- authentik ----------------------------------------------------------------------------
//
// Logging in at authentik means walking its "flow executor" API, which is what authentik's own
// login page does in the browser. In short:
//
//   authorize URL ──redirect──▶ login page for flow `<slug>`
//   flow executor:  GET → identification stage      (POST the username)
//                         → password stage          (POST the password)
//                         → redirects                (follow until it leaves the flow)
//   ──redirect──▶ authorize URL again, now logged in ──redirect──▶ scuba's /auth/callback?code=…

const AUTHENTIK: &str = "http://localhost:9000";
const SCUBA_CALLBACK: &str = "http://localhost:3000/auth/callback";

/// The login flow authentik sent us to.
struct LoginFlow {
    /// e.g. `default-authentication-flow`
    slug: String,
    /// The query string of the login page URL; the executor wants it passed along.
    query: String,
}

/// What the flow executor answers with.
enum FlowStep {
    /// The next stage to complete, as authentik's JSON description of it.
    Stage(Value),
    /// The flow is over (or hopping between stages): continue at this URL.
    Redirect(String),
}

/// A user's browser as seen by authentik: remembers cookies, asks for JSON, follows no redirects.
struct AuthentikBrowser {
    http: reqwest::Client,
}

impl AuthentikBrowser {
    fn new() -> Self {
        let json = [(reqwest::header::ACCEPT, "application/json".parse().unwrap())];
        Self {
            http: reqwest::Client::builder()
                .cookie_store(true)
                .redirect(reqwest::redirect::Policy::none())
                .default_headers(json.into_iter().collect())
                .build()
                .unwrap(),
        }
    }

    /// Log `username` in, starting from the authorize URL that scuba's `/login` redirected to.
    /// Returns scuba's callback as a path and query, ready to be given to scuba. `Err` says why
    /// authentik refused.
    async fn login(
        &self,
        authorize_url: &str,
        username: &str,
        password: &str,
    ) -> Result<String, String> {
        let flow = self.find_login_flow(authorize_url).await;
        let url_after_login = self.complete_flow(&flow, username, password).await?;
        self.follow_to_scuba(&url_after_login).await
    }

    /// Visit the authorize URL without a session: authentik redirects to its login page.
    async fn find_login_flow(&self, authorize_url: &str) -> LoginFlow {
        let response = self.http.get(authorize_url).send().await.unwrap();
        assert!(
            response.status().is_redirection(),
            "expected a redirect to the login page, got {}",
            response.status()
        );
        let location = absolute(response.headers()["location"].to_str().unwrap());

        // `<AUTHENTIK>/if/flow/<slug>/?<query>`
        let rest = location
            .strip_prefix(&format!("{AUTHENTIK}/if/flow/"))
            .unwrap_or_else(|| panic!("not sent to a login flow, but to {location}"));
        let (slug, query) = rest.split_once("/?").expect("login URL without a query");
        LoginFlow {
            slug: slug.to_owned(),
            query: query.to_owned(),
        }
    }

    /// Answer the flow's stages until it lets us go. Returns the URL it finally sends us to.
    async fn complete_flow(
        &self,
        flow: &LoginFlow,
        username: &str,
        password: &str,
    ) -> Result<String, String> {
        let executor = format!("{AUTHENTIK}/api/v3/flows/executor/{}/", flow.slug);
        let mut step = self.read_step(self.get_stage(&executor, flow).await).await;

        // A real flow has a handful of stages; the limit only stops a loop (e.g. a rejected
        // password makes authentik show the same stage again, forever).
        for _ in 0..10 {
            let stage = match step {
                FlowStep::Stage(stage) => stage,
                // Redirects within the flow are just "next stage, please".
                FlowStep::Redirect(url) if url.contains("/api/v3/flows/executor/") => {
                    step = self
                        .read_step(self.http.get(&url).send().await.unwrap())
                        .await;
                    continue;
                }
                // Back at the login page: authentik didn't let us in.
                FlowStep::Redirect(url) if url.contains("/if/flow/") => {
                    return Err("sent back to the login page".into());
                }
                // Anywhere else: the flow is done.
                FlowStep::Redirect(url) => return Ok(url),
            };

            let component = stage["component"].as_str().unwrap_or_default();
            step = match component {
                "ak-stage-identification" => {
                    let mut answer = json!({"component": component, "uid_field": username});
                    // Some flows ask for the password on the same screen.
                    if stage["password_fields"].as_bool().unwrap_or(false) {
                        answer["password"] = password.into();
                    }
                    self.read_step(self.post_answer(&executor, flow, &answer).await)
                        .await
                }
                "ak-stage-password" => {
                    let answer = json!({"component": component, "password": password});
                    self.read_step(self.post_answer(&executor, flow, &answer).await)
                        .await
                }
                // authentik sometimes states a redirect as a stage rather than as an HTTP redirect;
                // treat the two alike and let the redirect arms above handle it next time round.
                "xak-flow-redirect" => {
                    FlowStep::Redirect(absolute(stage["to"].as_str().expect("redirect target")))
                }
                "ak-stage-access-denied" => {
                    let reason = stage["error_message"].as_str().unwrap_or("access denied");
                    return Err(reason.to_owned());
                }
                other => panic!("unexpected login stage {other}: {stage}"),
            };
        }
        Err("login flow never finished (credentials rejected?)".into())
    }

    /// Now logged in, follow redirects (authorize → implicit consent → …) until authentik sends
    /// us back to scuba. Returns scuba's callback as a path and query.
    async fn follow_to_scuba(&self, start_url: &str) -> Result<String, String> {
        let mut url = start_url.to_owned();
        for _ in 0..10 {
            let response = self.http.get(&url).send().await.unwrap();
            assert!(
                response.status().is_redirection(),
                "expected a redirect at {url}, got {}",
                response.status()
            );
            let location = absolute(response.headers()["location"].to_str().unwrap());
            if location.starts_with(SCUBA_CALLBACK) {
                return Ok(callback_path_and_query(&location));
            }
            url = location;
        }
        Err("never reached scuba's callback".into())
    }

    // ---- small helpers for talking to the flow executor ----

    async fn get_stage(&self, executor: &str, flow: &LoginFlow) -> reqwest::Response {
        self.http
            .get(executor)
            .query(&[("query", flow.query.as_str())])
            .send()
            .await
            .unwrap()
    }

    async fn post_answer(
        &self,
        executor: &str,
        flow: &LoginFlow,
        answer: &Value,
    ) -> reqwest::Response {
        self.http
            .post(executor)
            .query(&[("query", flow.query.as_str())])
            .json(answer)
            .send()
            .await
            .unwrap()
    }

    /// A redirect means "continue there"; otherwise the body is the next stage as JSON.
    async fn read_step(&self, response: reqwest::Response) -> FlowStep {
        let status = response.status();
        if status.is_redirection() {
            let location = response.headers()["location"].to_str().unwrap();
            return FlowStep::Redirect(absolute(location));
        }
        let text = response.text().await.unwrap();
        let stage = serde_json::from_str(&text).unwrap_or_else(|e| {
            panic!("authentik answered {status} with non-JSON ({e}): {text:?}")
        });
        FlowStep::Stage(stage)
    }
}

/// Redirect locations may be relative to authentik; make them absolute.
fn absolute(location: &str) -> String {
    if location.starts_with("http") {
        location.to_owned()
    } else {
        format!("{AUTHENTIK}{location}")
    }
}

/// authentik redirects to scuba's callback both on success *and* on errors (`?error=…`).
/// Only a callback carrying a `code` is a login, so anything else fails the test loudly.
/// Returns the callback as a path and query.
fn callback_path_and_query(callback_url: &str) -> String {
    let url = reqwest::Url::parse(callback_url).unwrap();
    let query = url.query().unwrap_or_default();
    assert!(
        !query.contains("error="),
        "authentik redirected back with an error: {query}"
    );
    assert!(query.contains("code="), "callback without a code: {query}");
    format!("{}?{}", url.path(), query)
}

// ---- scuba against authentik --------------------------------------------------------------

/// scuba's login routes plus one protected page, `/whoami`, wired exactly like `main` does it
/// but against the real dev authentik.
async fn app(db: sqlx::SqlitePool) -> Router {
    // Show scuba's own log lines (login failures are logged, not displayed) when a test fails.
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let env = dev_env();
    let backend = Backend::new(
        db.clone(),
        &OidcConfig {
            issuer_url: env["OIDC_ISSUER_URL"].clone(),
            client_id: env["OIDC_CLIENT_ID"].clone(),
            client_secret: env["OIDC_CLIENT_SECRET"].clone(),
        },
        &env["SCUBA_PUBLIC_URL"],
    )
    .await
    .expect("OIDC discovery against the dev authentik (is it up, and did the blueprint apply?)");

    let protected = Router::new()
        .route(
            "/whoami",
            get(|auth: AuthSession| async move {
                auth.user
                    .map(|user| format!("{}|{}", user.email, user.display_name))
                    .unwrap_or_default()
            }),
        )
        .route_layer(login_required!(Backend, login_url = "/login"));

    let app = Router::new().merge(auth::router()).merge(protected);
    auth::with_auth_layers(app, db, backend, false)
}

#[tokio::test]
#[ignore = "needs the dev stack"]
async fn seeded_users_can_log_in_to_scuba_through_the_real_authentik() {
    let db = scuba::db::connect("sqlite::memory:").await.unwrap();
    let app = app(db.clone()).await;

    // (username, password, email) of the users the blueprint and bootstrap create
    let users = [
        ("alice", "dev", "alice@example.com"),
        ("bob", "dev", "bob@example.com"),
        ("akadmin", "admin", "admin@example.com"),
    ];
    for (username, password, email) in users {
        let mut browser = Browser::new(app.clone());

        // scuba sends the user to authentik …
        let login = browser.get("/login").await;
        assert!(login.status.is_redirection(), "{}", login.status);

        // … where they log in, and authentik sends them back with a code …
        let callback = AuthentikBrowser::new()
            .login(login.location(), username, password)
            .await
            .unwrap_or_else(|e| panic!("{username} could not log in: {e}"));

        // … which scuba accepts.
        let response = browser.get(&callback).await;
        assert!(
            response.status.is_redirection(),
            "callback for {username} answered {}: {}",
            response.status,
            response.body
        );
        assert_eq!(response.location(), "/");

        let whoami = browser.get("/whoami").await;
        assert_eq!(whoami.status, 200, "{username}");
        assert!(
            whoami.body.starts_with(email),
            "{username}: {}",
            whoami.body
        );
    }

    let (user_count,): (i64,) = sqlx::query_as("select count(*) from users")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(
        user_count, 3,
        "three authentik users should be three scuba users"
    );
}

#[tokio::test]
#[ignore = "needs the dev stack"]
async fn wrong_password_never_reaches_scuba() {
    let db = scuba::db::connect("sqlite::memory:").await.unwrap();
    let mut browser = Browser::new(app(db).await);

    let login = browser.get("/login").await;
    let outcome = AuthentikBrowser::new()
        .login(login.location(), "alice", "definitely-wrong")
        .await;

    // authentik reports a bad password in one of a few ways, depending on its flow settings
    let reason = outcome.expect_err("a wrong password must not produce a callback");
    assert!(
        ["Invalid password", "login page", "never finished"]
            .iter()
            .any(|expected| reason.contains(expected)),
        "unexpected refusal: {reason}"
    );

    // and scuba itself is still logged out
    assert!(browser.get("/whoami").await.status.is_redirection());
}
