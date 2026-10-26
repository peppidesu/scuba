//! `NetbirdClient` against an in-process fake of the NetBird API (`tests/common/netbird.rs`).
//!
//! Grouped by area: transport (auth, errors), peers, groups, group membership, policies, DNS.
//! The fake mimics the real API's quirks (`null` for empty lists, 404 for an unmatched name
//! filter, `PUT` replacing a group's members); `tests/dev_stack.rs` checks those against the
//! real thing.

mod common;

use std::time::Duration;

use common::netbird::{FakeNetbird, TOKEN};
use scuba::netbird::{Error, NetbirdClient};
use serde_json::json;

/// `["a", "b"]` as the `Vec<String>` the client API takes, for brevity.
fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// ---- transport ----

#[tokio::test]
async fn sends_token_auth_and_json_accept_on_every_request() {
    let nb = FakeNetbird::start().await;
    let c = nb.client();
    c.list_peers().await.unwrap();
    c.list_groups().await.unwrap();
    for r in nb.requests() {
        assert_eq!(
            r.authorization.as_deref(),
            Some(format!("Token {TOKEN}").as_str()),
            "{} {}",
            r.method,
            r.path
        );
        assert_eq!(r.accept.as_deref(), Some("application/json"));
    }
}

#[tokio::test]
async fn wrong_token_surfaces_as_api_error() {
    let nb = FakeNetbird::start().await;
    let c = NetbirdClient::new(nb.base.clone(), "nope".into()).unwrap();
    match c.list_peers().await {
        Err(Error::Api { status: 401, .. }) => {}
        other => panic!("expected 401 Api error, got {other:?}"),
    }
}

#[tokio::test]
async fn trailing_slash_on_base_url_is_tolerated() {
    let nb = FakeNetbird::start().await;
    let c = NetbirdClient::new(format!("{}/", nb.base), TOKEN.into()).unwrap();
    c.list_peers().await.unwrap();
    assert_eq!(nb.requests()[0].path, "/api/peers");
}

#[tokio::test]
async fn not_found_maps_to_not_found_variant() {
    let nb = FakeNetbird::start().await;
    assert!(matches!(
        nb.client().get_peer("missing").await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        nb.client().delete_group("missing").await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn other_failures_keep_status_and_body() {
    let nb = FakeNetbird::start().await;
    nb.fail_next(500, "boom");
    match nb.client().list_peers().await {
        Err(Error::Api { status, body }) => assert_eq!((status, body.as_str()), (500, "boom")),
        other => panic!("{other:?}"),
    }
    // and the client is still usable afterwards
    nb.client().list_peers().await.unwrap();
}

#[tokio::test]
async fn connection_failure_is_an_http_error() {
    let c = NetbirdClient::new("http://127.0.0.1:1".into(), TOKEN.into()).unwrap();
    assert!(matches!(c.list_peers().await, Err(Error::Http(_))));
}

// ---- peers ----

#[tokio::test]
async fn list_and_get_peers_with_their_groups() {
    let nb = FakeNetbird::start().await;
    nb.add_peer("p1", "laptop", Some("u1"));
    nb.add_peer("p2", "setup-key-box", None);
    nb.add_group("g-all", "All", &["p1", "p2"]);
    nb.add_group("g-mine", "mine", &["p1"]);

    let peers = nb.client().list_peers().await.unwrap();
    assert_eq!(peers.len(), 2);
    let p1 = peers.iter().find(|p| p.id == "p1").unwrap();
    assert_eq!(p1.user_id.as_deref(), Some("u1"));
    let mut names: Vec<_> = p1.groups.iter().map(|g| g.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["All", "mine"]);
    assert_eq!(peers.iter().find(|p| p.id == "p2").unwrap().user_id, None);

    assert_eq!(nb.client().get_peer("p1").await.unwrap().name, "laptop");
}

#[tokio::test]
async fn delete_peer_removes_it_everywhere() {
    let nb = FakeNetbird::start().await;
    nb.add_peer("p1", "laptop", Some("u1"));
    nb.add_group("g", "g", &["p1"]);
    nb.client().delete_peer("p1").await.unwrap();
    assert!(!nb.has_peer("p1"));
    assert!(nb.members("g").is_empty());
    assert!(matches!(
        nb.client().delete_peer("p1").await,
        Err(Error::NotFound)
    ));
}

// ---- groups ----

#[tokio::test]
async fn find_group_filters_by_exact_name() {
    let nb = FakeNetbird::start().await;
    nb.add_group("g1", "reef", &[]);
    nb.add_group("g2", "reef-extra", &[]);
    let c = nb.client();

    assert_eq!(c.find_group("reef").await.unwrap().unwrap().id, "g1");
    assert!(c.find_group("ree").await.unwrap().is_none());
    assert!(c.find_group("Reef").await.unwrap().is_none());
    let q = nb.requests_matching("GET", "/api/groups")[0]
        .query
        .clone()
        .unwrap();
    assert!(q.contains("name=reef"), "{q}");
}

#[tokio::test]
async fn create_and_delete_group() {
    let nb = FakeNetbird::start().await;
    let g = nb.client().create_group("team-a").await.unwrap();
    assert_eq!(g.name, "team-a");
    assert!(nb.has_group(&g.id));
    nb.client().delete_group(&g.id).await.unwrap();
    assert!(!nb.has_group(&g.id));
}

#[tokio::test]
async fn find_group_uses_the_encoded_name_param() {
    let nb = FakeNetbird::start().await;
    nb.add_group("g1", "a&b c", &[]);
    assert_eq!(
        nb.client().find_group("a&b c").await.unwrap().unwrap().id,
        "g1"
    );
}

// ---- set_peer_groups ----

#[tokio::test]
async fn adds_peer_to_managed_group_without_dropping_other_members() {
    let nb = FakeNetbird::start().await;
    for p in ["p1", "p2"] {
        nb.add_peer(p, p, Some("u"));
    }
    nb.add_group("g-mine", "mine", &["p2"]);

    nb.client()
        .set_peer_groups("p1", &ids(&["g-mine"]), &ids(&["g-mine"]))
        .await
        .unwrap();

    assert_eq!(nb.members("g-mine"), ["p1", "p2"]);
    let put = &nb.requests_matching("PUT", "/api/groups/g-mine")[0];
    let body = put.body.clone().unwrap();
    assert_eq!(body["name"], "mine", "PUT must carry the group's name");
    let mut sent: Vec<_> = body["peers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    sent.sort();
    assert_eq!(
        sent,
        ["p1", "p2"],
        "PUT must carry the full member list (replace semantics)"
    );
}

#[tokio::test]
async fn removes_peer_from_managed_group_only_when_not_desired() {
    let nb = FakeNetbird::start().await;
    for p in ["p1", "p2"] {
        nb.add_peer(p, p, Some("u"));
    }
    nb.add_group("g-a", "a", &["p1", "p2"]);
    nb.add_group("g-b", "b", &["p1", "p2"]);

    nb.client()
        .set_peer_groups("p1", &ids(&["g-b"]), &ids(&["g-a", "g-b"]))
        .await
        .unwrap();

    assert_eq!(nb.members("g-a"), ["p2"]);
    assert_eq!(nb.members("g-b"), ["p1", "p2"]);
}

#[tokio::test]
async fn never_touches_unmanaged_groups_in_either_direction() {
    let nb = FakeNetbird::start().await;
    nb.add_peer("p1", "p1", Some("u"));
    nb.add_group("g-all", "All", &["p1"]); // current membership, unmanaged: must stay
    nb.add_group("g-theirs", "theirs", &[]); // asked for, unmanaged: must not be joined
    nb.add_group("g-mine", "mine", &[]);

    nb.client()
        .set_peer_groups("p1", &ids(&["g-mine", "g-theirs"]), &ids(&["g-mine"]))
        .await
        .unwrap();

    assert_eq!(nb.members("g-all"), ["p1"]);
    assert!(nb.members("g-theirs").is_empty());
    assert_eq!(nb.members("g-mine"), ["p1"]);
    assert!(nb.requests_matching("PUT", "/api/groups/g-all").is_empty());
    assert!(
        nb.requests_matching("PUT", "/api/groups/g-theirs")
            .is_empty()
    );
}

#[tokio::test]
async fn already_matching_state_makes_no_writes() {
    let nb = FakeNetbird::start().await;
    nb.add_peer("p1", "p1", Some("u"));
    nb.add_group("g-in", "in", &["p1"]);
    nb.add_group("g-out", "out", &[]);

    nb.client()
        .set_peer_groups("p1", &ids(&["g-in"]), &ids(&["g-in", "g-out"]))
        .await
        .unwrap();

    assert!(
        nb.requests().iter().all(|r| r.method == "GET"),
        "{:?}",
        nb.requests()
    );
}

#[tokio::test]
async fn empty_desired_clears_all_managed_memberships() {
    let nb = FakeNetbird::start().await;
    nb.add_peer("p1", "p1", Some("u"));
    nb.add_group("g-a", "a", &["p1"]);
    nb.add_group("g-b", "b", &["p1"]);
    nb.client()
        .set_peer_groups("p1", &[], &ids(&["g-a", "g-b"]))
        .await
        .unwrap();
    assert!(nb.members("g-a").is_empty() && nb.members("g-b").is_empty());
}

#[tokio::test]
async fn unknown_peer_is_not_found_and_changes_nothing() {
    let nb = FakeNetbird::start().await;
    nb.add_group("g", "g", &[]);
    let r = nb
        .client()
        .set_peer_groups("ghost", &ids(&["g"]), &ids(&["g"]))
        .await;
    assert!(matches!(r, Err(Error::NotFound)));
    assert!(nb.requests().iter().all(|r| r.method == "GET"));
}

#[tokio::test]
async fn concurrent_updates_to_one_group_do_not_lose_members() {
    let nb = FakeNetbird::start().await;
    for p in ["p1", "p2", "p3"] {
        nb.add_peer(p, p, Some("u"));
    }
    nb.add_group("g", "g", &[]);
    // Slow reads make an unsynchronised read-modify-write lose updates reliably.
    nb.slow_group_reads(Duration::from_millis(40));
    let c = nb.client();
    let managed = ids(&["g"]);

    let (a, b, d) = tokio::join!(
        c.set_peer_groups("p1", &managed, &managed),
        c.set_peer_groups("p2", &managed, &managed),
        c.set_peer_groups("p3", &managed, &managed),
    );
    a.unwrap();
    b.unwrap();
    d.unwrap();

    assert_eq!(nb.members("g"), ["p1", "p2", "p3"]);
}

// ---- policies ----

#[tokio::test]
async fn create_policy_is_a_one_way_accept_rule() {
    let nb = FakeNetbird::start().await;
    let id = nb
        .client()
        .create_policy("all-to-reef", "g-all", "g-reef")
        .await
        .unwrap();
    assert!(!id.is_empty());

    let p = &nb.policies()[0];
    assert_eq!(p["enabled"], true);
    let rule = &p["rules"][0];
    assert_eq!(rule["action"], "accept");
    assert_eq!(
        rule["bidirectional"], false,
        "traffic must only flow source -> target"
    );
    assert_eq!(rule["protocol"], "all");
    assert_eq!(rule["sources"], json!(["g-all"]));
    assert_eq!(rule["destinations"], json!(["g-reef"]));
    assert_eq!(rule["enabled"], true);
}

#[tokio::test]
async fn delete_policy_removes_it() {
    let nb = FakeNetbird::start().await;
    let c = nb.client();
    let id = c.create_policy("p", "a", "b").await.unwrap();
    c.delete_policy(&id).await.unwrap();
    assert!(nb.policies().is_empty());
    assert!(matches!(c.delete_policy(&id).await, Err(Error::NotFound)));
}

// ---- dns ----

#[tokio::test]
async fn zone_lifecycle() {
    let nb = FakeNetbird::start().await;
    let c = nb.client();

    assert!(c.find_zone("corp").await.unwrap().is_none());
    let z = c.create_zone("corp", &ids(&["g-all"])).await.unwrap();
    assert_eq!(z.domain, "corp");
    assert_eq!(c.find_zone("corp").await.unwrap().unwrap().id, z.id);
    assert!(c.find_zone("corp2").await.unwrap().is_none());

    let sent = &nb.zones()[0];
    assert_eq!(sent["domain"], "corp");
    assert_eq!(sent["enabled"], true);
    assert_eq!(sent["distribution_groups"], json!(["g-all"]));
    assert!(
        sent["enable_search_domain"].is_boolean(),
        "field is required by the API"
    );

    c.delete_zone(&z.id).await.unwrap();
    assert!(c.find_zone("corp").await.unwrap().is_none());
}

#[tokio::test]
async fn record_lifecycle() {
    let nb = FakeNetbird::start().await;
    let c = nb.client();
    let z = c.create_zone("reef", &[]).await.unwrap();

    let r = c
        .create_record(&z.id, "web.foo.reef", "A", "100.64.0.9", 300)
        .await
        .unwrap();
    assert_eq!(
        (r.name.as_str(), r.rtype.as_str(), r.content.as_str(), r.ttl),
        ("web.foo.reef", "A", "100.64.0.9", 300)
    );
    let sent = &nb.records(&z.id)[0];
    assert_eq!(sent["type"], "A", "`rtype` must be sent as `type`");

    let listed = c.list_records(&z.id).await.unwrap();
    assert_eq!(listed.len(), 1);

    let upd = c
        .update_record(&z.id, &r.id, "web.foo.reef", "A", "100.64.0.10", 60)
        .await
        .unwrap();
    assert_eq!(
        (upd.id.as_str(), upd.content.as_str(), upd.ttl),
        (r.id.as_str(), "100.64.0.10", 60)
    );
    assert_eq!(nb.records(&z.id).len(), 1, "update must not duplicate");

    c.delete_record(&z.id, &r.id).await.unwrap();
    assert!(c.list_records(&z.id).await.unwrap().is_empty());
    assert!(matches!(
        c.delete_record(&z.id, &r.id).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn records_in_unknown_zone_are_not_found() {
    let nb = FakeNetbird::start().await;
    let r = nb
        .client()
        .create_record("nope", "a.b", "A", "1.1.1.1", 60)
        .await;
    assert!(matches!(r, Err(Error::NotFound)));
}

#[tokio::test]
async fn find_group_returns_none_when_the_api_answers_404_for_an_unmatched_name() {
    // The real NetBird answers 404 (not `[]`) for `GET /groups?name=<no match>`; the fake does too.
    let nb = FakeNetbird::start().await;
    assert!(
        nb.client()
            .find_group("nothing-here")
            .await
            .unwrap()
            .is_none()
    );
    nb.add_group("g1", "other", &[]);
    assert!(
        nb.client()
            .find_group("nothing-here")
            .await
            .unwrap()
            .is_none()
    );
}
