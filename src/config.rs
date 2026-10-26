//! Settings, read once at startup from environment variables.
//! See `dev/README.md` for a working local set.

use anyhow::Context;

/// Everything scuba needs from its environment.
pub struct Config {
    /// `SCUBA_LISTEN` — address to bind. Default `127.0.0.1:3000`.
    pub listen: String,
    /// `DATABASE_URL` — sqlite database file, e.g. `sqlite://scuba.db`. Default `sqlite://scuba.db`.
    pub database_url: String,
    /// `SCUBA_PUBLIC_URL` (required) — the address users reach scuba at, e.g.
    /// `https://scuba.example.com`; a trailing slash is dropped.
    /// Determines the OIDC redirect URI, and cookies are `Secure` when it starts with `https://`.
    pub public_url: String,
    pub oidc: OidcConfig,
    pub netbird: NetbirdConfig,
}

/// How to log users in with authentik. All three are required.
pub struct OidcConfig {
    /// `OIDC_ISSUER_URL` — for authentik `https://<host>/application/o/<slug>/`;
    /// the trailing slash matters.
    pub issuer_url: String,
    /// `OIDC_CLIENT_ID`
    pub client_id: String,
    /// `OIDC_CLIENT_SECRET`
    pub client_secret: String,
}

/// How to reach NetBird's management API. Both are required.
pub struct NetbirdConfig {
    /// `NETBIRD_API_URL` — e.g. `https://netbird.example.com`.
    pub api_url: String,
    /// `NETBIRD_API_TOKEN` — a personal access token of a user (ideally a service user)
    /// allowed to manage peers, groups, policies and DNS.
    pub api_token: String,
}

impl Config {
    /// Read the configuration from the process environment.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// [`Config::from_env`] with the environment abstracted away, so tests can pass a map.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let var = |name: &str| get(name).with_context(|| format!("missing env var {name}"));
        let var_or = |name: &str, default: &str| get(name).unwrap_or_else(|| default.to_owned());
        Ok(Self {
            listen: var_or("SCUBA_LISTEN", "127.0.0.1:3000"),
            database_url: var_or("DATABASE_URL", "sqlite://scuba.db"),
            public_url: var("SCUBA_PUBLIC_URL")?.trim_end_matches('/').to_owned(),
            oidc: OidcConfig {
                issuer_url: var("OIDC_ISSUER_URL")?,
                client_id: var("OIDC_CLIENT_ID")?,
                client_secret: var("OIDC_CLIENT_SECRET")?,
            },
            netbird: NetbirdConfig {
                api_url: var("NETBIRD_API_URL")?,
                api_token: var("NETBIRD_API_TOKEN")?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn full() -> HashMap<&'static str, &'static str> {
        HashMap::from([
            ("SCUBA_PUBLIC_URL", "https://scuba.example.com/"),
            (
                "OIDC_ISSUER_URL",
                "https://auth.example.com/application/o/scuba/",
            ),
            ("OIDC_CLIENT_ID", "id"),
            ("OIDC_CLIENT_SECRET", "secret"),
            ("NETBIRD_API_URL", "https://nb.example.com"),
            ("NETBIRD_API_TOKEN", "tok"),
        ])
    }

    fn load(m: &HashMap<&str, &str>) -> anyhow::Result<Config> {
        Config::from_lookup(|k| m.get(k).map(|v| v.to_string()))
    }

    #[test]
    fn loads_required_vars_and_applies_defaults() {
        let c = load(&full()).unwrap();
        assert_eq!(c.listen, "127.0.0.1:3000");
        assert_eq!(c.database_url, "sqlite://scuba.db");
        assert_eq!(c.oidc.client_id, "id");
        assert_eq!(c.netbird.api_token, "tok");
    }

    #[test]
    fn optional_vars_override_defaults() {
        let mut m = full();
        m.insert("SCUBA_LISTEN", "0.0.0.0:80");
        m.insert("DATABASE_URL", "sqlite://x.db");
        let c = load(&m).unwrap();
        assert_eq!(c.listen, "0.0.0.0:80");
        assert_eq!(c.database_url, "sqlite://x.db");
    }

    #[test]
    fn public_url_trailing_slash_is_trimmed() {
        assert_eq!(
            load(&full()).unwrap().public_url,
            "https://scuba.example.com"
        );
    }

    #[test]
    fn each_required_var_is_reported_by_name() {
        for name in full().keys() {
            let mut m = full();
            m.remove(name);
            let err = load(&m)
                .err()
                .unwrap_or_else(|| panic!("{name} should be required"));
            assert!(err.to_string().contains(name), "{err}");
        }
    }
}
