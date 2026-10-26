//! Database bootstrap, and the schema constraints the ownership rules will rely on: unique names
//! (so two people can't own the same DNS name or group), foreign keys, and cascading deletes.
//!
//! These talk to a real (in-memory) sqlite database, so they check what the migrations
//! actually create rather than what we think they create.

use scuba::db;
use sqlx::{Row, SqlitePool};

// ---- helpers ----

/// A fresh in-memory database with the schema applied.
async fn pool() -> SqlitePool {
    db::connect("sqlite::memory:").await.unwrap()
}

/// Create a user (the owner of things in the tests below) and return their id.
async fn user(db: &SqlitePool, sub: &str) -> i64 {
    db::users::upsert_by_sub(db, sub, &format!("{sub}@example.com"), sub)
        .await
        .unwrap()
        .id
}

/// Run one statement with text parameters.
async fn exec(db: &SqlitePool, sql: &'static str, binds: &[&str]) -> sqlx::Result<()> {
    let mut q = sqlx::query(sql);
    for b in binds {
        q = q.bind(*b);
    }
    q.execute(db).await.map(drop)
}

/// Insert a DNS claim without a NetBird zone. The raw result is returned so tests can assert
/// that the database refused it.
async fn claim(db: &SqlitePool, owner: i64, kind: &str, name: &str) -> sqlx::Result<()> {
    sqlx::query("insert into dns_claims (owner_id, kind, name) values (?, ?, ?)")
        .bind(owner)
        .bind(kind)
        .bind(name)
        .execute(db)
        .await
        .map(drop)
}

/// Insert a group and return its id; `owner: None` makes it a system group.
async fn group(db: &SqlitePool, name: &str, owner: Option<i64>, nb: &str) -> sqlx::Result<i64> {
    let r = sqlx::query(
        "insert into groups (name, owner_id, netbird_id) values (?, ?, ?) returning id",
    )
    .bind(name)
    .bind(owner)
    .bind(nb)
    .fetch_one(db)
    .await?;
    Ok(r.get(0))
}

/// Run a `select count(*) …` query.
async fn count(db: &SqlitePool, sql: &'static str) -> i64 {
    sqlx::query_as::<_, (i64,)>(sql)
        .fetch_one(db)
        .await
        .unwrap()
        .0
}

/// Was the statement rejected by a constraint (unique, foreign key or check)?
fn is_constraint_violation(error: &sqlx::Error) -> bool {
    let sqlx::Error::Database(db_error) = error else {
        return false;
    };
    db_error.is_unique_violation()
        || db_error.is_foreign_key_violation()
        || db_error.is_check_violation()
}

// ---- bootstrap ----

#[tokio::test]
async fn connect_creates_a_file_and_migrations_are_idempotent_and_persistent() {
    let path =
        std::env::temp_dir().join(format!("scuba-test-{}-{}.db", std::process::id(), line!()));
    let url = format!("sqlite://{}", path.display());
    let _ = std::fs::remove_file(&path);

    let first = db::connect(&url).await.unwrap();
    let id = user(&first, "sub-1").await;
    first.close().await;
    assert!(path.exists(), "create_if_missing");

    let second = db::connect(&url).await.unwrap(); // re-runs migrations
    assert_eq!(
        db::users::by_id(&second, id)
            .await
            .unwrap()
            .unwrap()
            .oidc_sub,
        "sub-1"
    );
    second.close().await;

    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
    }
}

#[tokio::test]
async fn all_expected_tables_exist() {
    let db = pool().await;
    for t in [
        "users",
        "dns_claims",
        "dns_records",
        "groups",
        "group_access",
        "sessions",
    ] {
        let n: (i64,) =
            sqlx::query_as("select count(*) from sqlite_master where type = 'table' and name = ?")
                .bind(t)
                .fetch_one(&db)
                .await
                .unwrap();
        assert_eq!(n.0, 1, "table {t}");
    }
}

#[tokio::test]
async fn foreign_keys_are_enforced() {
    let db = pool().await;
    let (on,): (i64,) = sqlx::query_as("pragma foreign_keys")
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(on, 1);
}

// ---- users ----

#[tokio::test]
async fn oidc_sub_is_unique() {
    let db = pool().await;
    exec(
        &db,
        "insert into users (oidc_sub, email, display_name) values (?, 'a', 'a')",
        &["s"],
    )
    .await
    .unwrap();
    let e = exec(
        &db,
        "insert into users (oidc_sub, email, display_name) values (?, 'b', 'b')",
        &["s"],
    )
    .await
    .unwrap_err();
    assert!(is_constraint_violation(&e), "{e}");
}

#[tokio::test]
async fn netbird_user_id_is_unique_when_set_but_may_be_null_many_times() {
    let db = pool().await;
    let a = user(&db, "a").await;
    let b = user(&db, "b").await; // both null: fine
    sqlx::query("update users set netbird_user_id = 'nb' where id = ?")
        .bind(a)
        .execute(&db)
        .await
        .unwrap();
    let e = sqlx::query("update users set netbird_user_id = 'nb' where id = ?")
        .bind(b)
        .execute(&db)
        .await
        .unwrap_err();
    assert!(is_constraint_violation(&e), "{e}");
}

// ---- dns claims ----

#[tokio::test]
async fn claim_names_are_globally_unique_across_owners() {
    let db = pool().await;
    let (a, b) = (user(&db, "a").await, user(&db, "b").await);
    claim(&db, a, "tld", "corp").await.unwrap();
    let e = claim(&db, b, "tld", "corp").await.unwrap_err();
    assert!(is_constraint_violation(&e), "{e}");
}

#[tokio::test]
async fn claim_kind_is_restricted() {
    let db = pool().await;
    let a = user(&db, "a").await;
    claim(&db, a, "reef", "foo.reef").await.unwrap();
    let e = claim(&db, a, "wildcard", "x").await.unwrap_err();
    assert!(is_constraint_violation(&e), "{e}");
}

#[tokio::test]
async fn claim_owner_must_exist() {
    let db = pool().await;
    let e = claim(&db, 9999, "tld", "corp").await.unwrap_err();
    assert!(is_constraint_violation(&e), "{e}");
}

#[tokio::test]
async fn deleting_a_claim_cascades_to_its_records() {
    let db = pool().await;
    let a = user(&db, "a").await;
    claim(&db, a, "tld", "corp").await.unwrap();
    let (cid,): (i64,) = sqlx::query_as("select id from dns_claims")
        .fetch_one(&db)
        .await
        .unwrap();
    for (fqdn, nb) in [("a.corp", "r1"), ("b.corp", "r2")] {
        sqlx::query("insert into dns_records (claim_id, fqdn, rtype, content, netbird_zone_id, netbird_id) values (?, ?, 'A', '1.1.1.1', 'z', ?)")
            .bind(cid)
            .bind(fqdn)
            .bind(nb)
            .execute(&db)
            .await
            .unwrap();
    }
    assert_eq!(count(&db, "select count(*) from dns_records").await, 2);
    sqlx::query("delete from dns_claims where id = ?")
        .bind(cid)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(count(&db, "select count(*) from dns_records").await, 0);
}

#[tokio::test]
async fn record_fqdns_are_unique() {
    let db = pool().await;
    let a = user(&db, "a").await;
    claim(&db, a, "tld", "corp").await.unwrap();
    let (cid,): (i64,) = sqlx::query_as("select id from dns_claims")
        .fetch_one(&db)
        .await
        .unwrap();
    let ins = |nb: &'static str| {
        let db = db.clone();
        async move {
            sqlx::query("insert into dns_records (claim_id, fqdn, rtype, content, netbird_zone_id, netbird_id) values (?, 'a.corp', 'A', '1.1.1.1', 'z', ?)")
                .bind(cid)
                .bind(nb)
                .execute(&db)
                .await
        }
    };
    ins("r1").await.unwrap();
    assert!(is_constraint_violation(&ins("r2").await.unwrap_err()));
}

#[tokio::test]
async fn tld_claim_zone_ids_are_unique_but_reef_claims_may_have_none() {
    let db = pool().await;
    let a = user(&db, "a").await;
    claim(&db, a, "reef", "x.reef").await.unwrap();
    claim(&db, a, "reef", "y.reef").await.unwrap(); // both NULL zone ids
    sqlx::query("insert into dns_claims (owner_id, kind, name, netbird_zone_id) values (?, 'tld', 't1', 'zone')").bind(a).execute(&db).await.unwrap();
    let e = sqlx::query("insert into dns_claims (owner_id, kind, name, netbird_zone_id) values (?, 'tld', 't2', 'zone')")
        .bind(a)
        .execute(&db)
        .await
        .unwrap_err();
    assert!(is_constraint_violation(&e), "{e}");
}

// ---- groups ----

#[tokio::test]
async fn group_names_and_netbird_ids_are_unique() {
    let db = pool().await;
    let a = user(&db, "a").await;
    group(&db, "team", Some(a), "nb1").await.unwrap();
    assert!(is_constraint_violation(
        &group(&db, "team", Some(a), "nb2").await.unwrap_err()
    ));
    assert!(is_constraint_violation(
        &group(&db, "other", Some(a), "nb1").await.unwrap_err()
    ));
}

#[tokio::test]
async fn system_groups_may_have_no_owner() {
    let db = pool().await;
    group(&db, "reef", None, "nb-reef").await.unwrap();
}

#[tokio::test]
async fn group_owner_must_exist() {
    let db = pool().await;
    assert!(is_constraint_violation(
        &group(&db, "g", Some(4242), "nb").await.unwrap_err()
    ));
}

#[tokio::test]
async fn access_rules_are_unique_per_pair_and_directional() {
    let db = pool().await;
    let a = user(&db, "a").await;
    let g1 = group(&db, "g1", Some(a), "n1").await.unwrap();
    let g2 = group(&db, "g2", Some(a), "n2").await.unwrap();
    let ins = |t: i64, s: i64, pol: &'static str| {
        let db = db.clone();
        async move {
            sqlx::query("insert into group_access (target_group_id, source_group_id, netbird_policy_id) values (?, ?, ?)")
                .bind(t)
                .bind(s)
                .bind(pol)
                .execute(&db)
                .await
        }
    };
    ins(g1, g2, "p1").await.unwrap();
    assert!(
        is_constraint_violation(&ins(g1, g2, "p2").await.unwrap_err()),
        "duplicate pair"
    );
    ins(g2, g1, "p3").await.unwrap(); // reverse direction is a distinct rule
    assert!(
        is_constraint_violation(&ins(g1, g1 + 100, "p4").await.unwrap_err()),
        "source must exist"
    );
}

#[tokio::test]
async fn deleting_a_group_removes_access_rules_in_both_roles() {
    let db = pool().await;
    let a = user(&db, "a").await;
    let g1 = group(&db, "g1", Some(a), "n1").await.unwrap();
    let g2 = group(&db, "g2", Some(a), "n2").await.unwrap();
    for (t, s, p) in [(g1, g2, "p1"), (g2, g1, "p2")] {
        sqlx::query("insert into group_access (target_group_id, source_group_id, netbird_policy_id) values (?, ?, ?)")
            .bind(t)
            .bind(s)
            .bind(p)
            .execute(&db)
            .await
            .unwrap();
    }
    sqlx::query("delete from groups where id = ?")
        .bind(g1)
        .execute(&db)
        .await
        .unwrap();
    assert_eq!(count(&db, "select count(*) from group_access").await, 0);
}
