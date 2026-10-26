//! Client for NetBird's management REST API — only the calls scuba needs.
//!
//! Authentication is a personal access token sent as `Authorization: Token …`. All ids are
//! NetBird's own (opaque strings); mapping them to scuba's database rows is the services' job.
//!
//! # NetBird quirks this client works around
//!
//! * **Empty collections are `null`**, not `[]` (e.g. a group with no peers). See [`null_as_default`].
//! * **A name filter with no match is a 404**, not an empty list (`GET /groups?name=x`).
//! * **`PUT /groups/{id}` replaces the whole member list**, so adding one peer means
//!   read-modify-write; [`NetbirdClient::set_peer_groups`] serialises those with a lock.
//! * **Zones hold records.** A TLD claim is its own zone; every `*.reef` name is a record in
//!   the single shared `reef` zone. A zone's `distribution_groups` decide who can resolve it.

use std::sync::Arc;

use reqwest::{Method, RequestBuilder, Response, StatusCode, header};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use tokio::sync::Mutex;

// ---- errors -------------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Couldn't reach NetBird, or its answer wasn't what we expected.
    #[error(transparent)]
    Http(#[from] reqwest::Error),

    /// HTTP 404: the peer/group/zone/record doesn't exist (or a name filter matched nothing).
    #[error("netbird: not found")]
    NotFound,

    /// Any other non-success answer, e.g. 401 for a bad token or 422 for an invalid request.
    #[error("netbird returned {status}: {body}")]
    Api { status: u16, body: String },
}

pub type Result<T> = std::result::Result<T, Error>;

// ---- what NetBird sends back --------------------------------------------------------------
// Only the fields scuba uses are listed; anything else in the JSON is ignored.

/// A group as it appears inside a peer, or a peer inside a group.
#[derive(Debug, Clone, Deserialize)]
pub struct GroupRef {
    pub id: String,
    pub name: String,
}

/// A device on the VPN.
#[derive(Debug, Clone, Deserialize)]
pub struct Peer {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub hostname: String,
    /// Its address on the VPN.
    #[serde(default)]
    pub ip: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub connected: bool,
    pub last_seen: Option<String>,
    /// The NetBird user that registered the device; absent for peers added with a setup key.
    /// This is how scuba knows whose device it is.
    pub user_id: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub groups: Vec<GroupRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    /// Who created it: `"api"`, `"jwt"` or `"integration"`. scuba only manages `"api"` groups.
    pub issued: Option<String>,
    #[serde(default, deserialize_with = "null_as_default")]
    pub peers: Vec<GroupRef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Zone {
    pub id: String,
    pub domain: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Record {
    pub id: String,
    pub name: String,
    /// `A`, `AAAA`, `CNAME`, … (called `type` on the wire).
    #[serde(rename = "type")]
    pub rtype: String,
    pub content: String,
    pub ttl: u32,
}

/// For responses where we only need the new object's id.
#[derive(Deserialize)]
struct Id {
    id: String,
}

/// Request body for creating or updating a DNS record.
#[derive(Serialize)]
struct RecordBody<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    rtype: &'a str,
    content: &'a str,
    ttl: u32,
}

/// Treats JSON `null` like a missing field. Plain `#[serde(default)]` only handles *missing*
/// and would reject the `null` NetBird sends for empty collections.
fn null_as_default<'de, D, T>(deserializer: D) -> std::result::Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

// ---- the client ---------------------------------------------------------------------------

/// Cheap to clone; all clones share one HTTP connection pool and one group lock.
#[derive(Clone)]
pub struct NetbirdClient {
    http: reqwest::Client,
    /// Base URL without trailing slash, e.g. `https://netbird.example.com`.
    base: String,
    /// Held while changing group membership; see the module docs. Assumes a single scuba instance.
    group_lock: Arc<Mutex<()>>,
}

impl NetbirdClient {
    /// `base` is NetBird's address (the `/api` prefix is added here); `token` a personal
    /// access token. Fails only if the token contains characters illegal in an HTTP header.
    pub fn new(base: String, token: String) -> anyhow::Result<Self> {
        let mut auth = header::HeaderValue::from_str(&format!("Token {token}"))?;
        auth.set_sensitive(true); // keeps it out of debug logs
        let mut headers = header::HeaderMap::new();
        headers.insert(header::AUTHORIZATION, auth);
        headers.insert(
            header::ACCEPT,
            header::HeaderValue::from_static("application/json"),
        );

        let http = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            group_lock: Default::default(),
        })
    }

    // ---- plumbing ----

    /// Start a request to `/api{path}`.
    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.http
            .request(method, format!("{}/api{path}", self.base))
    }

    /// Turn non-success HTTP statuses into [`Error`]s.
    async fn check(response: Response) -> Result<Response> {
        let status = response.status();
        if status.is_success() {
            Ok(response)
        } else if status == StatusCode::NOT_FOUND {
            Err(Error::NotFound)
        } else {
            Err(Error::Api {
                status: status.as_u16(),
                body: response.text().await.unwrap_or_default(),
            })
        }
    }

    /// Send, check, and parse the JSON body as `T`.
    async fn send_json<T: DeserializeOwned>(request: RequestBuilder) -> Result<T> {
        Ok(Self::check(request.send().await?).await?.json().await?)
    }

    /// Send and check, ignoring any body.
    async fn send(request: RequestBuilder) -> Result<()> {
        Self::check(request.send().await?).await.map(drop)
    }

    // ---- peers ----

    /// Every peer on the VPN, whoever owns it. Filter on [`Peer::user_id`] for one user's devices.
    pub async fn list_peers(&self) -> Result<Vec<Peer>> {
        Self::send_json(self.request(Method::GET, "/peers")).await
    }

    pub async fn get_peer(&self, id: &str) -> Result<Peer> {
        Self::send_json(self.request(Method::GET, &format!("/peers/{id}"))).await
    }

    /// Remove a device from the VPN (it can re-register).
    pub async fn delete_peer(&self, id: &str) -> Result<()> {
        Self::send(self.request(Method::DELETE, &format!("/peers/{id}"))).await
    }

    // ---- groups ----

    pub async fn list_groups(&self) -> Result<Vec<Group>> {
        Self::send_json(self.request(Method::GET, "/groups")).await
    }

    pub async fn get_group(&self, id: &str) -> Result<Group> {
        Self::send_json(self.request(Method::GET, &format!("/groups/{id}"))).await
    }

    /// The group with exactly this name, if there is one.
    pub async fn find_group(&self, name: &str) -> Result<Option<Group>> {
        let request = self
            .request(Method::GET, "/groups")
            .query(&[("name", name)]);
        let groups: Vec<Group> = match Self::send_json(request).await {
            Ok(groups) => groups,
            // NetBird answers 404, not an empty list, when nothing matches.
            Err(Error::NotFound) => return Ok(None),
            Err(e) => return Err(e),
        };
        Ok(groups.into_iter().find(|g| g.name == name))
    }

    /// Create an empty group.
    pub async fn create_group(&self, name: &str) -> Result<Group> {
        Self::send_json(
            self.request(Method::POST, "/groups")
                .json(&json!({ "name": name })),
        )
        .await
    }

    pub async fn delete_group(&self, id: &str) -> Result<()> {
        Self::send(self.request(Method::DELETE, &format!("/groups/{id}"))).await
    }

    /// Put a peer in exactly the groups `desired`, **among the groups in `managed`**.
    ///
    /// `managed` is the set of group ids the caller may change for this user. Everything outside
    /// it is left alone in both directions, even if it appears in `desired`: the built-in `All`
    /// group, other people's groups, groups created by the identity provider, and so on.
    /// So a request can never put a device into a group it isn't entitled to, nor pull it out of
    /// one it shouldn't be able to leave.
    pub async fn set_peer_groups(
        &self,
        peer_id: &str,
        desired: &[String],
        managed: &[String],
    ) -> Result<()> {
        // Each group change below is read-modify-write; don't let two interleave.
        let _guard = self.group_lock.lock().await;

        let peer = self.get_peer(peer_id).await?;
        let current: Vec<&str> = peer.groups.iter().map(|g| g.id.as_str()).collect();

        let is_desired = |group_id: &str| desired.iter().any(|d| d == group_id);
        let is_current = |group_id: &str| current.contains(&group_id);

        let to_join = managed
            .iter()
            .filter(|id| is_desired(id) && !is_current(id));
        let to_leave = managed
            .iter()
            .filter(|id| !is_desired(id) && is_current(id));

        for group_id in to_join {
            self.edit_members(group_id, |members| {
                if !members.iter().any(|m| m == peer_id) {
                    members.push(peer_id.to_owned());
                }
            })
            .await?;
        }
        for group_id in to_leave {
            self.edit_members(group_id, |members| members.retain(|m| m != peer_id))
                .await?;
        }
        Ok(())
    }

    /// Read a group's member ids, let `edit` change them, write the list back.
    /// The caller must hold `group_lock`.
    async fn edit_members(
        &self,
        group_id: &str,
        edit: impl FnOnce(&mut Vec<String>),
    ) -> Result<()> {
        let group = self.get_group(group_id).await?;
        let mut members: Vec<String> = group.peers.into_iter().map(|p| p.id).collect();
        edit(&mut members);
        let body = json!({ "name": group.name, "peers": members });
        Self::send(
            self.request(Method::PUT, &format!("/groups/{group_id}"))
                .json(&body),
        )
        .await
    }

    // ---- policies ----

    /// Allow traffic from `source_group` to `target_group` (one way, any protocol and port).
    /// Returns the new policy's id; keep it to be able to remove the rule later.
    pub async fn create_policy(
        &self,
        name: &str,
        source_group: &str,
        target_group: &str,
    ) -> Result<String> {
        let body = json!({
            "name": name,
            "description": "managed by scuba",
            "enabled": true,
            "rules": [{
                "name": name,
                "enabled": true,
                "action": "accept",
                "bidirectional": false,
                "protocol": "all",
                "sources": [source_group],
                "destinations": [target_group],
            }],
        });
        let created: Id =
            Self::send_json(self.request(Method::POST, "/policies").json(&body)).await?;
        Ok(created.id)
    }

    pub async fn delete_policy(&self, id: &str) -> Result<()> {
        Self::send(self.request(Method::DELETE, &format!("/policies/{id}"))).await
    }

    // ---- dns ----

    /// The zone for exactly this domain, if there is one.
    pub async fn find_zone(&self, domain: &str) -> Result<Option<Zone>> {
        let zones: Vec<Zone> = Self::send_json(self.request(Method::GET, "/dns/zones")).await?;
        Ok(zones.into_iter().find(|z| z.domain == domain))
    }

    /// Create a zone. Peers in `distribution_groups` can resolve its names; pass the id of the
    /// built-in `All` group to make it resolvable for everyone.
    pub async fn create_zone(&self, domain: &str, distribution_groups: &[String]) -> Result<Zone> {
        let body = json!({
            "name": domain,
            "domain": domain,
            "enabled": true,
            "enable_search_domain": false,
            "distribution_groups": distribution_groups,
        });
        Self::send_json(self.request(Method::POST, "/dns/zones").json(&body)).await
    }

    /// Delete a zone together with its records.
    pub async fn delete_zone(&self, id: &str) -> Result<()> {
        Self::send(self.request(Method::DELETE, &format!("/dns/zones/{id}"))).await
    }

    pub async fn list_records(&self, zone_id: &str) -> Result<Vec<Record>> {
        Self::send_json(self.request(Method::GET, &format!("/dns/zones/{zone_id}/records"))).await
    }

    /// Add a record to a zone. `fqdn` must be the zone's domain or a name beneath it.
    pub async fn create_record(
        &self,
        zone_id: &str,
        fqdn: &str,
        rtype: &str,
        content: &str,
        ttl: u32,
    ) -> Result<Record> {
        let body = RecordBody {
            name: fqdn,
            rtype,
            content,
            ttl,
        };
        Self::send_json(
            self.request(Method::POST, &format!("/dns/zones/{zone_id}/records"))
                .json(&body),
        )
        .await
    }

    /// Replace a record's type, content and TTL. Like the other updates, it overwrites everything,
    /// so pass the values you want to keep as well.
    pub async fn update_record(
        &self,
        zone_id: &str,
        record_id: &str,
        fqdn: &str,
        rtype: &str,
        content: &str,
        ttl: u32,
    ) -> Result<Record> {
        let body = RecordBody {
            name: fqdn,
            rtype,
            content,
            ttl,
        };
        Self::send_json(
            self.request(
                Method::PUT,
                &format!("/dns/zones/{zone_id}/records/{record_id}"),
            )
            .json(&body),
        )
        .await
    }

    pub async fn delete_record(&self, zone_id: &str, record_id: &str) -> Result<()> {
        Self::send(self.request(
            Method::DELETE,
            &format!("/dns/zones/{zone_id}/records/{record_id}"),
        ))
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_ignores_unknown_fields_and_defaults_optional_ones() {
        let p: Peer = serde_json::from_value(json!({
            "id": "p1", "name": "laptop", "connected": true, "user_id": "u1",
            "ip": "100.64.0.1", "some_new_field": {"x": 1},
            "groups": [{"id": "g1", "name": "All", "peers_count": 3, "issued": "api"}],
        }))
        .unwrap();
        assert_eq!(
            (p.id.as_str(), p.user_id.as_deref(), p.connected),
            ("p1", Some("u1"), true)
        );
        assert_eq!(p.groups[0].name, "All");
        assert_eq!(p.hostname, "");
    }

    #[test]
    fn peer_without_user_id_is_none() {
        let p: Peer = serde_json::from_value(json!({"id": "p1", "name": "srv"})).unwrap();
        assert_eq!(p.user_id, None);
        assert!(p.groups.is_empty());
    }

    #[test]
    fn group_members_default_empty() {
        let g: Group = serde_json::from_value(json!({"id": "g", "name": "n"})).unwrap();
        assert!(g.peers.is_empty() && g.issued.is_none());
    }

    #[test]
    fn record_body_serializes_type_field() {
        let v = serde_json::to_value(RecordBody {
            name: "a.corp",
            rtype: "A",
            content: "1.2.3.4",
            ttl: 60,
        })
        .unwrap();
        assert_eq!(
            v,
            json!({"name": "a.corp", "type": "A", "content": "1.2.3.4", "ttl": 60})
        );
    }

    #[test]
    fn record_deserializes_type_field() {
        let r: Record = serde_json::from_value(
            json!({"id":"r","name":"a.corp","type":"AAAA","content":"::1","ttl":30}),
        )
        .unwrap();
        assert_eq!(r.rtype, "AAAA");
    }

    #[test]
    fn empty_collections_may_be_null_as_the_real_api_sends_them() {
        // captured from a real NetBird: `GET /api/groups` for the built-in `All` group
        let g: Vec<Group> = serde_json::from_str(
            r#"[{"id":"dbfslpkhvvs000fek5e0","issued":"api","name":"All","peers":null,"peers_count":0,"resources":null,"resources_count":0}]"#,
        )
        .unwrap();
        assert!(g[0].peers.is_empty());
        let p: Peer = serde_json::from_str(r#"{"id":"p","name":"n","groups":null}"#).unwrap();
        assert!(p.groups.is_empty());
    }
    #[test]
    fn new_rejects_tokens_that_cannot_be_a_header() {
        assert!(NetbirdClient::new("http://x".into(), "bad\ntoken".into()).is_err());
    }
}
