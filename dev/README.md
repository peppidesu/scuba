# Local development stack

Runs the two services scuba depends on, so you can develop without any external infrastructure:

| Service   | URL                     | What it's for                                             | Login |
|-----------|-------------------------|-----------------------------------------------------------|-------|
| authentik | http://localhost:9000   | Identity provider scuba users log in with (OIDC)           | `akadmin` / `admin`, plus `alice` / `dev`, `bob` / `dev` |
| NetBird   | http://localhost:8080   | The VPN control plane scuba manages (dashboard + REST API) | `admin@example.com` / `admin-password` |

> **Dev only.** Fixed passwords and secrets, plain HTTP, no hardening. Never expose these ports.

## Start

```sh
docker compose -f dev/compose.yaml up -d
dev/setup.sh                      # once: creates the NetBird owner + API token, writes dev/scuba.env
set -a; . dev/scuba.env; set +a   # load scuba's configuration into your shell
cargo run                         # http://localhost:3000
```

Log in to scuba with `alice` / `dev` (or `bob`, or `akadmin`). authentik creates the `scuba`
OIDC application on first start from `dev/authentik/blueprints/scuba.yaml` — nothing to click through.

If you use direnv, add `dotenv_if_exists dev/scuba.env` to your `.envrc` instead of sourcing by hand.

## How it fits together

- **scuba → authentik**: OIDC authorization-code flow. Issuer `http://localhost:9000/application/o/scuba/`,
  client id `scuba`, redirect `http://localhost:3000/auth/callback`. If you run scuba on another
  host/port, set `SCUBA_URL` before `docker compose up` (and `SCUBA_PUBLIC_URL` for scuba) so the redirect URI matches.
- **scuba → NetBird**: REST API at `http://localhost:8080/api` using the API token from `setup.sh`.
- **NetBird** runs as the combined server (management + signal + relay + STUN) with its built-in login, so it
  needs no external IdP. A small Caddy proxy puts the dashboard, API, OIDC, gRPC and relay on a single
  origin (`:8080`); `3478/udp` (STUN) is published directly.
- `setup.sh` deletes NetBird's built-in allow-all `Default` policy: scuba is deny-by-default (only `All → reef` is open), and that policy would hide access-control bugs.
- scuba links people across the two systems **by email**: use the same email in authentik and NetBird for a
  person (the seeded admin already does: `admin@example.com`).

## Seeing devices

scuba lists a user's NetBird peers. Peers need to register as a *user* (not with a setup key) to belong to that user:

```sh
netbird up --management-url http://localhost:8080   # opens a browser login; use a NetBird dashboard user
```

Create more NetBird users in the dashboard (Team → Users) to try multi-user scenarios.

## Everyday commands

```sh
docker compose -f dev/compose.yaml logs -f authentik-server netbird-server
docker compose -f dev/compose.yaml down        # stop, keep data
docker compose -f dev/compose.yaml down -v     # stop and wipe everything (then: rm dev/scuba.env; dev/setup.sh)
```

Pin different versions with `AUTHENTIK_TAG`, `NETBIRD_TAG`, `NETBIRD_DASHBOARD_TAG`.

## Testing against the real stack

`tests/dev_stack.rs` runs scuba's NetBird client and its OIDC login against these real services
(not the in-process fakes the normal tests use). It is ignored by default so `cargo test` needs no Docker:

```sh
cargo test --test dev_stack -- --ignored --test-threads=1
```

It logs `alice`, `bob` and `akadmin` in through authentik, checks that a wrong password never reaches scuba, and
exercises groups, policies, DNS zones and records on NetBird (cleaning up after itself).

> **Note:** until the `services` layer is implemented, `cargo run` stops at startup (`ensure_reef` is still a stub).
> The live tests cover the same login path in the meantime.
