# DNS Publisher Records (human names instead of hex)

> **Priority 02 · Tier 1 — adoption:** names are marketing; kills the hex-soup UX; cheap and offline-testable. Tracked in [#16](https://github.com/safonas/ocid/issues/16) (mirrored: `rad:f211a100`). **SHIPPED** — see Status below.

## Context
Registry paths cannot contain uppercase and publisher identity is a raw
64-hex endpoint id, so references look like
`127.0.0.1:5050/3f9a…c1/app:1.0` until the user creates local aliases.
A domain name is a far better global handle: whoever controls a DNS zone
publishes a record binding the zone to their publisher key, e.g.

    podman pull 127.0.0.1:5050/images.radicle.xyz/radicle:latest

The domain appears as the first path segment (same slot as `<hex>` today),
so the loopback-registry trick is unchanged — the registry just needs to
resolve `images.radicle.xyz` to a publisher id. Trust carries over from the
existing model: releases are already signed by that key, so the record only
has to convince us *which* key owns the name; every artifact it publishes is
self-verifying afterwards.

## Proposal
- Record format: TXT at `_ocid.<zone>`, e.g.
  `v=ocid1 k=<did:key> ts=<unix> sig=<hex>` where `sig` covers the canonical
  payload (version, zone, key, timestamp) **with the publisher key itself**.
  A record signed by K, served from the zone, proves mutual consent: the
  zone admin points the name at K and only K's owner could have signed it.
- Path resolution: extend the hybrid scheme — first segment contains a dot
  and has a `_ocid` TXT record → publisher = record key (after signature
  verification + pin check); fall through to existing alias/implicit rules
  otherwise.
- Client hardening (plain DNS is spoofable):
  - **TOFU pinning**: first verified resolution is pinned in a trust store
    (`$OCID_HOME/dns-pins.json`); later records for the same zone must match
    the pin or be rejected loudly.
  - Verify the record signature always; require fresh `ts` (reject records
    older than a policy window); use DNSSEC validation opportunistically
    when the resolver reports it.
- Policy/CLI: `ocictl follow images.radicle.xyz/radicle`, `ocictl seed
  images.radicle.xyz/radicle:1.0`, and `ocictl resolve <domain>` to inspect
  the mapping, signature, and pin status. Domain forms accepted wherever
  hex/`did:key`/aliases are today (policy.toml, refs, `ocictl track`).
- Reuse what is already in the tree: hickory-resolver arrives with iroh 1.2,
  and n0's iroh-dns discovery already publishes Ed25519-signed TXT records
  (pkarr packets). Consider a pkarr-compatible record so a domain can
  double as a **discovery anchor**: extra fields (`relay=`, `seed=`) let
  `ocictl connect images.radicle.xyz` bootstrap the swarm from the same
  record — one DNS name then carries identity *and* reachability.
- Docs/tests: update the naming section in `docs/DESIGN.md` and the trust
  boundaries (DNS is an untrusted lookup, pins are the root); e2e case with
  a local zone file + dnsmasq; unit tests for record parse/verify/pin/
  rollback handling.

## Why this priority
Cheap relative to its impact: no new infrastructure, mostly resolver +
naming + pin-store work, testable offline with a local zone. It removes the
biggest UX wart (hex soup) and makes the edge-Kubernetes story presentable —
Flux `OCIRepository` URLs like `images.example.com/team/app` look normal.
Slot it directly after the Podman Desktop extension and alongside/before
`06-edge-kubernetes-flux.md`, which it complements.

## Status

**SHIPPED in v0.8.0-rc.1** 2026-10-09 — gh#96 merged to main (9e1903c).
All 8 acceptance criteria from gh#16 met:

- [x] Record type: parse/serialize, canonical payload, Ed25519 verify, reject bad sig (8 unit tests in `dns.rs`)
- [x] TOFU pin store: pin on first use, mismatch rejected loudly, explicit re-pin flow (unit + e2e)
- [x] Freshness window enforcement (unit + e2e stale-record leg)
- [x] Path-resolution precedence incl. domain branch (`domain_segments_fall_through_to_existing_rules` unit test + e2e)
- [x] Offline resolution from pins (pin survives daemon restart, e2e)
- [x] `ocictl resolve` + domain refs in follow/seed/pin (e2e CLI legs)
- [x] e2e: dnsmasq zone; two-node pull by domain, follow by domain (97 assertions)
- [x] `docs/DESIGN.md` naming diagram + trust boundaries; README naming table gains the domain row; `docs/dns-publisher-names.md` user guide

Feature breakdown (gh#96, 6 commits on `feat/dns-records`):

- core: record format + TOFU pins + `is_domain_name` gate + `ocictl dns-record`
- daemon: hickory resolver with TTL/negative cache, registry `resolve()`
  DNS pre-step, `/_ocid/dns/resolve` + `/_ocid/dns/unpin`,
  `ocictl resolve` / `ocictl dns-unpin`
- policy domain forms: follow/seed/pin accept domains; `policy.toml`
  keeps the domain form and records the resolved key in a `[dns]` mapping
  table (evaluation never needs DNS at load time)
- e2e: dnsmasq zone — pull by name, hijack rejection, stale rejection,
  pin-survives-restart, rotation via unpin, policy domain forms, CLI
  surface (97 assertions green)
- docs: DESIGN.md naming diagram + trust boundaries, README naming
  table + CLI reference, `docs/dns-publisher-names.md` user guide
- CodeQL: scoped `rust/cleartext-logging` exclusion for
  `crates/ocictl/src/main.rs` (the CLI prints the node's own public key
  on purpose — it is the product, not a secret)

Follow-up (post-v0.8.0): pkarr/discovery-hint evaluation (one DNS
name for identity + reachability — `relay=`/`seed=` fields so
`ocictl connect images.example.com` bootstraps the swarm from the
same record).
