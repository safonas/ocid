# pkarr/DHT bootstrap — one name, identity + reachability

> **Priority 16 · Tier 2 — adoption:** kills the connection-ticket UX; one
> publisher name (or bare key) is enough to find and dial a peer. Builds on
> shipped DNS publisher names (02) and iroh's pkarr plumbing, which already
> runs inside every ocid daemon today.
> Tracked in [#117](https://github.com/safonas/ocid/issues/117) (mirrored: `rad:ca0d4b50`).

## Status

**Design approved; slices 1 + 2 implemented and verified (one branch, PR
gh#123) — including a live `E2E_ONLINE=1` run of the pkarr path.**
Target release: **0.9.0** (after 0.8.0 final lands on main).

Implementation slices:

- **Slice 1** — connect by bare key: `ocictl connect <did:key | 64-hex>`,
  same for `POST /_ocid/peers` and `ocid --peer`; explicit `pkarr` config
  + status surface. **Implemented.**
- **Slice 2** — DNS record extension: optional `relay=`/`seed=` fields
  (v=ocid1 kept, spec overwritten in place — exactly one record exists in
  the world, the publisher re-publishes), `ocictl connect <domain>`,
  `ocictl dns-record --relay/--seed`, dnsmasq e2e leg (offline, node with
  mDNS+pkarr off reaches the publisher through the signed hints and the
  seed introducer), opt-in `E2E_ONLINE=1` leg for live pkarr (local runs
  only, never CI). **Implemented.**

## Context

Two facts make this cheap:

1. **iroh already does pkarr for us.** Every ocid endpoint is built with
   `presets::N0`, which installs `PkarrPublisher::n0_dns()` +
   `PkarrResolver::n0_dns()`: the daemon publishes its signed endpoint
   address (relay URL + direct addresses) to n0.computer's pkarr endpoint,
   which re-publishes into the BitTorrent mainline DHT; and it resolves any
   bare `EndpointId` the same way. So "dial by bare key" is plumbing: we
   currently only accept tickets, never bare ids.
2. **The DNS record (02) already proves which key owns a name.** It just
   carries no reachability. Adding optional hints to the same signed record
   makes `ocictl connect images.example.com` work offline of any ticket.

The ticket remains the out-of-band bootstrap of last resort, and mDNS
covers the LAN (`--no-relay`). This feature is about the remaining case:
internet-separated strangers.

## Proposal

### 1. Connect by bare key (Slice 1)

Accept `did:key:z6Mk…` and 64-hex ids everywhere a ticket is accepted
today:

- `ocictl connect <ticket | did:key | 64-hex>`
- `POST /_ocid/peers` `{"ticket": …}` → generalize to `{"peer": …}`
  (old key kept as alias for one version)
- `ocid --peer <…>` bootstrap flags

Mechanics: parse the id, build an `EndpointAddr` with **only** the id, and
let iroh dial — `Endpoint::connect` consults the installed
`PkarrResolver` when no addresses are known. No ocid protocol changes.

**Config** (`config.toml`):

```toml
pkarr = "on"   # default. "off": do not add PkarrPublisher/PkarrResolver
               # to the endpoint — no address publishing, no lookup.
```

Today pkarr publishing is implicit and invisible (part of the N0 preset)
and stays on even under `--no-relay` (which only disables relay servers).
The flag makes it explicit and gives privacy-sensitive users an off
switch. `ocictl status` / `GET /_ocid/status` report it.

### 2. Reachability hints in the DNS record (Slice 2)

**Format decision (owner's call): keep `v=ocid1` and overwrite the spec
in place.** Exactly one record exists in the world (`_ocid.ocid.dev`);
the publisher re-publishes it with the new fields. No version bump, no
migration. Consequence to note in docs: records carrying the new optional
fields fail to parse on pre-0.9 daemons (`unknown record field`) —
acceptable with a single coordinated re-publish; the fields are optional
so a minimal record keeps working everywhere.

Extended record (optional fields, all covered by the signature):

```
v=ocid1 k=<64-hex> ts=<unix> [relay=<url>] [seed=<64-hex>] sig=<128-hex>
```

- canonical payload becomes `ocid1 <zone> <k> <ts> [relay] [seed]` —
  hints are signed, so DNS cannot invent or swap them.
- `relay=<url>` — a relay the publisher is reachable through; used as the
  `relay_url` hint of the `EndpointAddr` when dialing.
- `seed=<64-hex>` — an introducer: an ocid peer that is a member of the
  publisher's swarm topic. Dial it first; iroh-gossip topic membership
  then connects us to the publisher (neighbors), and ocid's swarm
  bootstrap does the rest (this is exactly how `--peer` bootstraps work
  today).

**`ocictl connect <domain>`**: resolve `_ocid.<zone>` (existing verified
lookup + TOFU pin) → dial the key with the record's hints, falling back to
plain pkarr resolution when the record carries none. Either way: no
ticket.

**`ocictl dns-record <zone>`** gains `--relay <url>` and
`--seed <peer-id>` flags. **Decision: explicit flags only** — the node's
current relay is never embedded implicitly. A relay URL baked into a
30-day record can go stale (the node may switch relays, or drop to
`--no-relay`), and the record needs monthly re-signing for `ts` freshness
anyway — re-stating the hints then is free, and a minimal record keeps
working everywhere. Docs must carry a note: `relay=`/`seed=` hints can go
stale until the record is re-published; peers treat them as hints, not
guarantees.

### 3. Tests

- **Unit** (`ocid-core::dns`): parse/serialize/verify with 0–2 hint fields;
  unknown-field rejection stays; canonical-payload regression (a v1-parsed
  record with hints must not verify against the hintless payload); TXT
  string splitting at 255 bytes with byte-exact reassembly. **All in.**
- **e2e (offline, dnsmasq)**: node C's record served with both hints
  (relay deliberately bogus — hints are best-effort and must not break
  the dial) → a fresh node with mDNS and pkarr off, bootstrapped to the
  seed by ticket, runs `ocictl connect <zone>` and ends up as C's gossip
  neighbor, then pulls C's release by domain name. Plus a
  `pkarr = "off"` status leg and a multi-string TXT serving check.
  **In (sections 10–11).**
- **Online (opt-in, `E2E_ONLINE=1`, never in CI)**: G runs with relays +
  pkarr on but mDNS off; A (relays on — iroh pkarr records publish the
  relay URL only, so a relay-disabled dialer cannot use them) connects by
  bare hex and reaches G as a neighbor through live pkarr (publish +
  resolve via n0.computer into the mainline DHT, dial via the home relay).
  Runs locally when invoked explicitly; the build server never touches
  the live DHT. **In and passing (125/125 with the leg enabled).**
  Notable iroh behaviors the leg works around, for future reference:
  the first pkarr publish is empty (relay URL follows once the home relay
  connection settles, ~seconds), and gossip retries a failed dial only
  once — so the leg waits for the record to carry the relay URL before
  connecting (host-side probe of the relay's HTTP API, key in z-base-32).

### 4. Documentation

- `docs/dns-publisher-names.md` — "Bootstrap from the same record"
  section; connect-by-domain + connect-by-key examples.
- `docs/DESIGN.md` — discovery story (pkarr publisher/resolver + mDNS +
  tickets + DNS hints) and trust boundaries (pkarr records are
  self-certifying: the DHT is untrusted storage, the signature is the
  boundary — same argument as the DNS record, with the key already known).
- `README.md` — connect section: all four forms, the `pkarr` config row.
- `docs/architecture/components.md` — discovery table gains the pkarr row.
- `todos/02` — follow-up note marked moved here.

## Future reference (deliberately out of scope for 0.9.0)

Evaluated and parked — notes for whoever picks this up:

- **Self-hosted pkarr relay** (`iroh-dns-server`): run the pkarr HTTP +
  DNS endpoint + mainline DHT re-publisher yourself; config gets
  `pkarr_relay = "https://pkarr.example.com"` (falls back to n0.computer).
  Fits the air-gap/organizational deployments (03/08) — the same binary
  also serves `n0.computer`-style DNS discovery. Small, mostly config.
- **Direct mainline DHT publishing** (no HTTP relay hop): publish the
  signed address packet straight into the DHT from the daemon via a
  mainline crate. Removes the n0.computer dependency for publishing;
  costs a background re-publish loop (DHT entries expire in ~2 h) and a
  new dependency surface. Only worth it if n0.computer becomes a
  bottleneck or a trust concern.
- **Domain-free handles**: pkarr makes the *key itself* the namespace, so
  `did:key` alone could be the whole public identity — no DNS zone
  needed. UX call, not a technical one; revisit with the 02 follow-ups.

## Why this priority

The two-node demo already works on a LAN (mDNS) and with tickets; the
stranger-on-the-internet case is the last bootstrap gap. Everything here
is either exposing machinery iroh already runs (slice 1) or one optional
field pair in a record format with exactly one live instance (slice 2) —
high adoption impact, low risk, no protocol break.
