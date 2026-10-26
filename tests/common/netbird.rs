//! An in-process fake of the NetBird management API: just the endpoints scuba uses, kept in
//! memory, served on a random local port.
//!
//! It aims to behave like the real thing where that matters. A peer's `groups` are derived from
//! group membership, `PUT /groups/{id}` replaces the member list, empty collections are `null`,
//! and a group-name filter with no match is a 404. (The real-API behaviour was captured from a
//! real NetBird; see `tests/dev_stack.rs`.)
//!
//! Tests set things up with the `add_*` methods, run the client under test against
//! [`FakeNetbird::client`], then inspect the result with the assertion helpers or the list of
//! recorded requests.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use serde_json::{Value, json};

/// The only API token the fake accepts.
pub const TOKEN: &str = "test-token";

/// One request the fake received, for assertions about what the client sent.
#[derive(Clone, Debug)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    /// The JSON body, if there was one.
    pub body: Option<Value>,
    pub authorization: Option<String>,
    pub accept: Option<String>,
}

// ---- the fake's data ----------------------------------------------------------------------

struct FakePeer {
    id: String,
    name: String,
    /// `None` models a peer registered with a setup key.
    user_id: Option<String>,
}

struct FakeGroup {
    id: String,
    name: String,
    /// Ids of the peers in the group.
    members: Vec<String>,
}

#[derive(Default)]
struct World {
    peers: Vec<FakePeer>,
    groups: Vec<FakeGroup>,
    policies: Vec<Value>,
    zones: Vec<Value>,
    /// Records by the id of the zone they are in.
    records: HashMap<String, Vec<Value>>,
    requests: Vec<Recorded>,
    /// Answer the next request with this status and body instead of handling it.
    fail_next: Option<(u16, String)>,
    group_read_delay: Duration,
    next_id: u32,
}

impl World {
    fn new_id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    fn peer_name(&self, peer_id: &str) -> String {
        self.peers
            .iter()
            .find(|p| p.id == peer_id)
            .map(|p| p.name.clone())
            .unwrap_or_default()
    }

    fn peer_json(&self, peer: &FakePeer) -> Value {
        let groups: Vec<Value> = self
            .groups
            .iter()
            .filter(|g| g.members.contains(&peer.id))
            .map(|g| json!({"id": g.id, "name": g.name, "peers_count": g.members.len(), "issued": "api"}))
            .collect();
        let mut json = json!({
            "id": peer.id, "name": peer.name, "hostname": peer.name,
            "ip": "100.64.0.1", "os": "linux", "connected": true,
            "last_seen": "2026-01-01T00:00:00Z",
            "groups": null_if_empty(groups),
            "ssh_enabled": false, "approval_required": false,
        });
        if let Some(user_id) = &peer.user_id {
            json["user_id"] = json!(user_id);
        }
        json
    }

    fn group_json(&self, group: &FakeGroup) -> Value {
        let peers: Vec<Value> = group
            .members
            .iter()
            .map(|id| json!({"id": id, "name": self.peer_name(id)}))
            .collect();
        json!({
            "id": group.id, "name": group.name, "issued": "api",
            "peers_count": group.members.len(), "peers": null_if_empty(peers),
            "resources": null,
        })
    }
}

/// Like the real API, an empty list is sent as `null`.
fn null_if_empty(items: Vec<Value>) -> Value {
    if items.is_empty() {
        Value::Null
    } else {
        Value::Array(items)
    }
}

/// The peer ids in a request body's `"peers": ["id", …]`.
fn peer_ids(body: &Value) -> Vec<String> {
    body["peers"]
        .as_array()
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

type Shared = Arc<Mutex<World>>;

// ---- the public face: what tests call ---------------------------------------------------------

pub struct FakeNetbird {
    /// `http://127.0.0.1:<port>`
    pub base: String,
    world: Shared,
}

impl FakeNetbird {
    pub async fn start() -> Self {
        let world: Shared = Default::default();
        let app = Router::new()
            .route("/api/peers", get(list_peers))
            .route("/api/peers/{id}", get(get_peer).delete(delete_peer))
            .route("/api/groups", get(list_groups).post(create_group))
            .route(
                "/api/groups/{id}",
                get(get_group).put(put_group).delete(delete_group),
            )
            .route("/api/policies", post(create_policy))
            .route("/api/policies/{id}", delete(delete_policy))
            .route("/api/dns/zones", get(list_zones).post(create_zone))
            .route("/api/dns/zones/{id}", delete(delete_zone))
            .route(
                "/api/dns/zones/{id}/records",
                get(list_records).post(create_record),
            )
            .route(
                "/api/dns/zones/{zone}/records/{record}",
                put(update_record).delete(delete_record),
            )
            .layer(middleware::from_fn_with_state(
                world.clone(),
                record_and_gate,
            ))
            .with_state(world.clone());

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { base, world }
    }

    /// A real [`scuba::netbird::NetbirdClient`] pointed at this fake, with the right token.
    pub fn client(&self) -> scuba::netbird::NetbirdClient {
        scuba::netbird::NetbirdClient::new(self.base.clone(), TOKEN.into()).unwrap()
    }

    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    // ---- set up ----

    pub fn add_peer(&self, id: &str, name: &str, user_id: Option<&str>) {
        self.world().peers.push(FakePeer {
            id: id.into(),
            name: name.into(),
            user_id: user_id.map(Into::into),
        });
    }

    /// `members` are peer ids.
    pub fn add_group(&self, id: &str, name: &str, members: &[&str]) {
        self.world().groups.push(FakeGroup {
            id: id.into(),
            name: name.into(),
            members: members.iter().map(|m| m.to_string()).collect(),
        });
    }

    /// The next request, whatever it is, fails with this status and body.
    pub fn fail_next(&self, status: u16, body: &str) {
        self.world().fail_next = Some((status, body.into()));
    }

    /// Make reading a group slow. Combined with concurrent updates this makes a lost update
    /// (two read-modify-write cycles interleaving) show up reliably.
    pub fn slow_group_reads(&self, delay: Duration) {
        self.world().group_read_delay = delay;
    }

    // ---- inspect ----

    /// Sorted ids of the peers in a group.
    pub fn members(&self, group_id: &str) -> Vec<String> {
        let world = self.world();
        let mut members = world
            .groups
            .iter()
            .find(|g| g.id == group_id)
            .map(|g| g.members.clone())
            .unwrap_or_default();
        members.sort();
        members
    }

    pub fn has_peer(&self, id: &str) -> bool {
        self.world().peers.iter().any(|p| p.id == id)
    }

    pub fn has_group(&self, id: &str) -> bool {
        self.world().groups.iter().any(|g| g.id == id)
    }

    /// The policies as stored, i.e. as the client sent them (plus an `id`).
    pub fn policies(&self) -> Vec<Value> {
        self.world().policies.clone()
    }

    pub fn zones(&self) -> Vec<Value> {
        self.world().zones.clone()
    }

    pub fn records(&self, zone_id: &str) -> Vec<Value> {
        self.world()
            .records
            .get(zone_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.world().requests.clone()
    }

    pub fn requests_matching(&self, method: &str, path: &str) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|r| r.method == method && r.path == path)
            .collect()
    }
}

// ---- cross-cutting behaviour ---------------------------------------------------------------

/// Runs before every handler: record the request, apply `fail_next`, check the token.
async fn record_and_gate(State(world): State<Shared>, request: Request, next: Next) -> Response {
    let (parts, body) = request.into_parts();
    let body_bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    let header = |name: &str| {
        parts
            .headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(String::from)
    };
    let authorization = header("authorization");

    {
        let mut world = world.lock().unwrap();
        world.requests.push(Recorded {
            method: parts.method.to_string(),
            path: parts.uri.path().into(),
            query: parts.uri.query().map(Into::into),
            body: serde_json::from_slice(&body_bytes).ok(),
            authorization: authorization.clone(),
            accept: header("accept"),
        });
        if let Some((status, body)) = world.fail_next.take() {
            return (StatusCode::from_u16(status).unwrap(), body).into_response();
        }
    }

    if authorization.as_deref() != Some(&format!("Token {TOKEN}")) {
        return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
    }

    // The body was consumed above to record it; hand the handler a fresh copy.
    let request = Request::from_parts(parts, axum::body::Body::from(body_bytes));
    next.run(request).await
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"message": "not found", "code": 404})),
    )
        .into_response()
}

/// The answer to a delete: `{}` if something was removed, a 404 if there was nothing there.
fn deleted(something_was_removed: bool) -> Response {
    if something_was_removed {
        Json(json!({})).into_response()
    } else {
        not_found()
    }
}

// ---- peers --------------------------------------------------------------------------------

async fn list_peers(State(world): State<Shared>) -> Json<Value> {
    let world = world.lock().unwrap();
    Json(Value::Array(
        world.peers.iter().map(|p| world.peer_json(p)).collect(),
    ))
}

async fn get_peer(State(world): State<Shared>, Path(id): Path<String>) -> Response {
    let world = world.lock().unwrap();
    match world.peers.iter().find(|p| p.id == id) {
        Some(peer) => Json(world.peer_json(peer)).into_response(),
        None => not_found(),
    }
}

async fn delete_peer(State(world): State<Shared>, Path(id): Path<String>) -> Response {
    let mut world = world.lock().unwrap();
    let count_before = world.peers.len();
    world.peers.retain(|p| p.id != id);
    for group in &mut world.groups {
        group.members.retain(|member| member != &id);
    }
    deleted(world.peers.len() < count_before)
}

// ---- groups -------------------------------------------------------------------------------

async fn list_groups(
    State(world): State<Shared>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let world = world.lock().unwrap();
    let name_filter = query.get("name");
    let matches: Vec<Value> = world
        .groups
        .iter()
        .filter(|g| name_filter.is_none_or(|name| name == &g.name))
        .map(|g| world.group_json(g))
        .collect();

    // Like the real API: a name filter that matches nothing is a 404, not an empty list.
    if name_filter.is_some() && matches.is_empty() {
        return not_found();
    }
    Json(Value::Array(matches)).into_response()
}

async fn get_group(State(world): State<Shared>, Path(id): Path<String>) -> Response {
    let delay = world.lock().unwrap().group_read_delay;
    let snapshot = {
        let world = world.lock().unwrap();
        world
            .groups
            .iter()
            .find(|g| g.id == id)
            .map(|g| world.group_json(g))
    };
    // Sleep *after* taking the snapshot, so a concurrent writer really can change the group
    // between our read and whatever the caller does next with it.
    tokio::time::sleep(delay).await;
    match snapshot {
        Some(group) => Json(group).into_response(),
        None => not_found(),
    }
}

async fn create_group(State(world): State<Shared>, Json(body): Json<Value>) -> Response {
    let mut world = world.lock().unwrap();
    let group = FakeGroup {
        id: world.new_id("grp-"),
        name: body["name"].as_str().unwrap_or_default().into(),
        members: peer_ids(&body),
    };
    let json = world.group_json(&group);
    world.groups.push(group);
    Json(json).into_response()
}

/// Replaces the group's name and member list, as the real API does.
async fn put_group(
    State(world): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    let mut world = world.lock().unwrap();
    let Some(index) = world.groups.iter().position(|g| g.id == id) else {
        return not_found();
    };
    world.groups[index].name = body["name"].as_str().unwrap_or_default().into();
    world.groups[index].members = peer_ids(&body);
    Json(world.group_json(&world.groups[index])).into_response()
}

async fn delete_group(State(world): State<Shared>, Path(id): Path<String>) -> Response {
    let mut world = world.lock().unwrap();
    let count_before = world.groups.len();
    world.groups.retain(|g| g.id != id);
    deleted(world.groups.len() < count_before)
}

// ---- policies -----------------------------------------------------------------------------

async fn create_policy(State(world): State<Shared>, Json(mut policy): Json<Value>) -> Json<Value> {
    let mut world = world.lock().unwrap();
    policy["id"] = json!(world.new_id("pol-"));
    world.policies.push(policy.clone());
    Json(policy)
}

async fn delete_policy(State(world): State<Shared>, Path(id): Path<String>) -> Response {
    let mut world = world.lock().unwrap();
    let count_before = world.policies.len();
    world.policies.retain(|p| p["id"] != json!(id));
    deleted(world.policies.len() < count_before)
}

// ---- dns zones and records ------------------------------------------------------------------

async fn list_zones(State(world): State<Shared>) -> Json<Value> {
    Json(Value::Array(world.lock().unwrap().zones.clone()))
}

async fn create_zone(State(world): State<Shared>, Json(mut zone): Json<Value>) -> Json<Value> {
    let mut world = world.lock().unwrap();
    zone["id"] = json!(world.new_id("zone-"));
    world.zones.push(zone.clone());
    Json(zone)
}

async fn delete_zone(State(world): State<Shared>, Path(id): Path<String>) -> Response {
    let mut world = world.lock().unwrap();
    let count_before = world.zones.len();
    world.zones.retain(|z| z["id"] != json!(id));
    world.records.remove(&id); // a zone takes its records with it
    deleted(world.zones.len() < count_before)
}

async fn list_records(State(world): State<Shared>, Path(zone_id): Path<String>) -> Json<Value> {
    let records = world
        .lock()
        .unwrap()
        .records
        .get(&zone_id)
        .cloned()
        .unwrap_or_default();
    Json(Value::Array(records))
}

async fn create_record(
    State(world): State<Shared>,
    Path(zone_id): Path<String>,
    Json(mut record): Json<Value>,
) -> Response {
    let mut world = world.lock().unwrap();
    if !world.zones.iter().any(|z| z["id"] == json!(zone_id)) {
        return not_found();
    }
    record["id"] = json!(world.new_id("rec-"));
    world
        .records
        .entry(zone_id)
        .or_default()
        .push(record.clone());
    Json(record).into_response()
}

async fn update_record(
    State(world): State<Shared>,
    Path((zone_id, record_id)): Path<(String, String)>,
    Json(mut new_record): Json<Value>,
) -> Response {
    let mut world = world.lock().unwrap();
    let existing = world
        .records
        .get_mut(&zone_id)
        .and_then(|records| records.iter_mut().find(|r| r["id"] == json!(record_id)));
    let Some(existing) = existing else {
        return not_found();
    };
    new_record["id"] = json!(record_id);
    *existing = new_record.clone();
    Json(new_record).into_response()
}

async fn delete_record(
    State(world): State<Shared>,
    Path((zone_id, record_id)): Path<(String, String)>,
) -> Response {
    let mut world = world.lock().unwrap();
    let Some(records) = world.records.get_mut(&zone_id) else {
        return not_found();
    };
    let count_before = records.len();
    records.retain(|r| r["id"] != json!(record_id));
    deleted(records.len() < count_before)
}
