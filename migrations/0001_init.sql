-- NetBird is the source of truth for peers, groups and policies.
-- This DB only records *ownership* and the mapping to NetBird object ids.

CREATE TABLE users (
    id              INTEGER PRIMARY KEY,
    oidc_sub        TEXT NOT NULL UNIQUE,     -- authentik `sub`
    email           TEXT NOT NULL,
    display_name    TEXT NOT NULL,
    netbird_user_id TEXT UNIQUE               -- links peers (peer.user_id) to this user
) STRICT;

-- `name` is stored lowercase, without trailing dot.
--   kind='tld':  name = "corp"       (owner owns "corp" and everything under it)
--   kind='reef': name = "foo.reef"   (owner owns "foo.reef" and everything under it)
-- Overlap (equal / ancestor / descendant) between *different* owners is rejected
-- in policy::dns; UNIQUE only catches exact duplicates as a last line of defence.
CREATE TABLE dns_claims (
    id          INTEGER PRIMARY KEY,
    owner_id    INTEGER NOT NULL REFERENCES users(id),
    kind        TEXT NOT NULL CHECK (kind IN ('tld', 'reef')),
    name        TEXT NOT NULL UNIQUE,
    netbird_zone_id TEXT UNIQUE,              -- kind='tld': the zone we created; NULL for 'reef'
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
) STRICT;

-- Records created under a claim, mirrored to NetBird's DNS API.
CREATE TABLE dns_records (
    id              INTEGER PRIMARY KEY,
    claim_id        INTEGER NOT NULL REFERENCES dns_claims(id) ON DELETE CASCADE,
    fqdn            TEXT NOT NULL UNIQUE,
    rtype           TEXT NOT NULL,
    content         TEXT NOT NULL,
    netbird_zone_id TEXT NOT NULL,            -- tld claim's zone, or the shared `reef` zone
    netbird_id      TEXT NOT NULL
) STRICT;

CREATE TABLE groups (
    id          INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    owner_id    INTEGER REFERENCES users(id),   -- NULL only for system groups (reef)
    netbird_id  TEXT NOT NULL UNIQUE,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
) STRICT;

-- "traffic from source -> target is allowed". Only target's owner may create/delete.
CREATE TABLE group_access (
    id                INTEGER PRIMARY KEY,
    target_group_id   INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    source_group_id   INTEGER NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    netbird_policy_id TEXT NOT NULL UNIQUE,
    UNIQUE (target_group_id, source_group_id)
) STRICT;
