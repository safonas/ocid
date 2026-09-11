# ocid — design

`ocid` is a local-first, peer-to-peer alternative to centralized OCI
registries: every node runs a loopback OCI registry that standard tooling
(`podman`/`docker`/`oras`) talks to unchanged, while peers exchange signed,
content-addressed releases directly.

The design borrows Radicle's model — keys are identities, signed records
carry trust, replication follows a local policy — mapped onto OCI images:

| Radicle | ocid |
|---|---|
| Ed25519 node id / `did:key` | same key: iroh `EndpointId` + publisher id |
| repository (`rad:z…`) | image `<publisher>/<name>` |
| signed refs | signed **release** record per `name:tag` |
| git objects | OCI blobs (manifest, config, layers, referrers) |
| `git-remote-rad` | embedded OCI v2 registry on `localhost:5050` |
| `rad seed` / `rad follow` | `ocictl seed` / `follow` / `pin` with a retention window |
| gossip announcements | `iroh-gossip`, one topic per publisher |
| fetch protocol | `ocid/sync/1` + `iroh-blobs` |
| `rad node` / `rad` | `ocid` (daemon) / `ocictl` (CLI) |

## Components

```mermaid
flowchart LR
    subgraph host["your machine"]
        podman["podman / docker / oras"]
        cli["ocictl"]
        tui["ocitop (TUI)"]
        prom["Prometheus"]
        subgraph node["ocid (daemon)"]
            reg["OCI v2 registry<br/>127.0.0.1:5050/v2"]
            ctl["control API & SSE<br/>/_ocid/*"]
            met["/metrics<br/>OpenMetrics"]
            core["Node<br/>identity · policy · windows · fetch · GC"]
            store["Store<br/>iroh-blobs FsStore + JSON index"]
            gossip["iroh-gossip<br/>swarm + publisher topics"]
            sync["ocid/sync/1"]
            blobs["iroh-blobs"]
            mdns["mDNS lookup<br/>_ocid._udp"]
            ep["iroh Endpoint<br/>(QUIC, NAT traversal)"]
        end
    end
    swarm(("peers"))

    podman -- "HTTP /v2" --> reg
    cli -- "HTTP /_ocid" --> ctl
    tui -- "HTTP /_ocid & SSE" --> ctl
    cli -. "offline: reads index" .-> store
    prom --> met
    reg --> core
    ctl --> core
    core --> store
    core --> gossip
    core --> sync
    core --> blobs
    gossip --> ep
    sync --> ep
    blobs --> ep
    mdns --> ep
    ep <--> swarm
```

Crates: `ocid-core` (library: identity, config/policy, OCI types, release
records, index, API DTOs, optional client) — `ocid` (daemon) — `ocictl` (CLI) — `ocitop` (TUI dashboard).

## Identity and naming

```mermaid
flowchart LR
    sk["secret.key<br/>(ed25519)"] --> pk["public key"]
    pk --> hex["hex (64)<br/>EndpointId<br/>used in registry paths"]
    pk --> did["did:key:z6Mk…<br/>CLI / display"]
    pk --> sig["signs releases"]
    pk --> quic["authenticates QUIC<br/>connections"]
    pk --> topic["derives the publisher's<br/>gossip topic"]
```

Repository path resolution (hybrid scheme):

```mermaid
flowchart TD
    in["/v2/&lt;repo&gt;/…"] --> q1{"first segment<br/>is 64-hex?"}
    q1 -- yes --> explicit["publisher = hex<br/>name = rest"]
    q1 -- no --> q2{"whole repo<br/>is an image alias?"}
    q2 -- yes --> ialias["alias → publisher/name"]
    q2 -- no --> q3{"first segment<br/>is a publisher alias?"}
    q3 -- yes --> palias["publisher = alias target<br/>name = rest"]
    q3 -- no --> me["publisher = me<br/>name = repo"]
```

Only the key holder can write into a publisher namespace: pushes to
`<other>/…` are refused with 403.

## Data model

```mermaid
classDiagram
    class Release {
        ReleasePayload payload
        hex signature
        verify()
        supersedes(other)
    }
    class ReleasePayload {
        u8 version
        PublisherId publisher
        String name
        String tag
        BlobRef manifest
        BlobRef[] blobs
        u64 timestamp
        Referrer[] referrers
    }
    class BlobRef {
        Digest digest_sha256
        Hash hash_blake3
        u64 size
        String media_type
    }
    class Referrer {
        Digest digest
        String media_type
        u64 size
        String? artifact_type
        Map annotations
    }
    Release --> ReleasePayload
    ReleasePayload --> BlobRef : manifest
    ReleasePayload --> "*" BlobRef : blobs
    ReleasePayload --> "*" Referrer : referrers
```

* The **signature** covers the canonical JSON of the payload and is made by
  `payload.publisher`; anyone can verify it offline. Records travel as the
  exact JSON bytes that were signed.
* `blobs` lists everything the manifest references transitively (config,
  layers, nested manifests for indexes, referrer manifests and their blobs),
  each with **both** digests. sha256 is what OCI tooling expects; BLAKE3 is
  what iroh-blobs streams and verifies incrementally.
* Newer `timestamp` for the same `(publisher, name, tag)` supersedes.
  Timestamps are seconds; within one second the greater tag sorts newer.
* `referrers` are manifests whose `subject` is this release's manifest
  (signatures, SBOMs, attestations). Attaching one re-signs and republishes
  the release, so referrers replicate with the image they describe.

On disk:

```
blobs/                                      iroh-blobs FsStore (BLAKE3)
index/digests/<sha256>.json                 BlobRef  (sha256 → blake3)
index/releases/<pub>/<name>/<tag>.json      Release
index/referrers/<subject-hex>/<hex>.json    Referrer (for the referrers API)
```

## Publish

```mermaid
sequenceDiagram
    participant P as podman
    participant R as registry (A)
    participant S as Store (A)
    participant G as gossip topic(A)
    P->>R: POST/PATCH/PUT blobs
    R->>S: sha256 verify, import → blake3, index, pin
    P->>R: PUT /v2/app/manifests/1.0
    R->>R: resolve "app" → (me, app)
    R->>S: check all referenced blobs present
    R->>S: store manifest blob
    R->>R: build ReleasePayload (+ known referrers), sign
    R->>S: put release
    R->>G: broadcast Announcement{publisher, name, tag, digest, ts}
    R-->>P: 201 Created
```

A manifest with a `subject` takes a side path: it is stored, recorded under
`index/referrers/<subject>`, and every own release whose manifest is the
subject is re-signed with the new referrer and re-announced.

## Policy and retention windows

```mermaid
flowchart LR
    subgraph rules["policy.toml"]
        f["[[follow]] publisher, mode"]
        s["[[seed]] ref, mode"]
        st["[[seed]] ref:tag"]
        p["pin = [ref:tag]"]
    end
    f --> w
    s --> w
    st --> w
    p --> w
    w["Window(publisher, name)<br/>full | last:N | tags"]
    w --> sel["select(known releases)<br/>newest first, tag tie-break"]
    sel --> keep["wanted set"]
```

* A **mode** is `latest` (= `last:1`), `last:N` or `full`. Several rules for
  one image combine to the widest window; tagged seeds and pins add fixed
  tags.
* `select_wanted` is evaluated against everything the node knows about an
  image — local releases plus what peers' inventories and announcements
  report — so a node with `last:2` fetches the two newest and nothing else.
* `enforce_window` runs after every replication and on policy reload: releases
  outside the window are removed and their blobs unpinned. Own releases and
  pins are never touched.
* Everything the policy does not name is **cache**: on-demand pulls and
  leftovers of removed rules. GC keeps cache for `gc_grace_secs` after it was
  fetched, then drops it. `ocictl gc --force` ignores the grace period.

## Gossip topics

```mermaid
flowchart TB
    subgraph swarm["swarm topic  blake3('ocid/swarm/v1')"]
        A1[A]; B1[B]; C1[C]
    end
    subgraph ta["topic(A)  blake3('ocid/publisher/v1/' ‖ A)"]
        A2[A]; B2["B (follows A)"]
    end
    subgraph tc["topic(C)"]
        C2[C]; B3["B (pins C/x:1)"]
    end
```

* Every node joins the **swarm topic**. It carries no announcements; it
  supplies neighbors (`NeighborUp` → inventory sync) and bootstrap peers.
* A node subscribes to **its own topic** and to the topic of every publisher
  its policy follows, seeds or pins; `ocictl` reloads the policy in the daemon,
  which joins/leaves topics accordingly. Announcements are broadcast on the
  publisher's topic only, so nodes receive exactly what they can act on.
* A newly reachable peer (swarm neighbor, mDNS, `ocictl connect`) is
  introduced to every subscribed topic; peers not on a topic ignore the join.
* Announcements from a different publisher than the topic's owner are dropped.

## Replicate (policy-driven)

```mermaid
sequenceDiagram
    participant A as node A (publisher)
    participant B as node B (follows A, last:2)
    A-)B: gossip Announcement on topic(A)
    B->>B: select_wanted(known ∪ announced) → wanted?
    B->>A: sync GetRelease(publisher, name, tag)
    A-->>B: Release (signed JSON bytes)
    B->>B: verify signature
    loop each BlobRef not present
        B->>A: iroh-blobs download(blake3), protected from GC
        A-->>B: verified stream (bao)
        B->>B: verify sha256 + size, index, pin blob
    end
    B->>B: put release
    B->>B: enforce_window → prune oldest, unpin blobs
```

Same logic runs on `NeighborUp`, on mDNS discovery and on `ocictl sync`,
starting from the peer's **Inventory** instead of a single announcement —
this catches up after downtime.

## Pull on demand

```mermaid
sequenceDiagram
    participant P as podman
    participant C as registry (C)
    participant X as peers (neighbors, known, publisher)
    P->>C: GET /v2/&lt;A&gt;/alpine/manifests/3
    C->>C: local release complete? no
    loop candidate peers, until found
        C->>X: sync GetRelease
        X-->>C: Release | None
    end
    C->>X: download blobs (shuffled providers)
    C->>C: verify, index, store release (cache)
    C-->>P: manifest (+ Docker-Content-Digest)
    P->>C: GET blobs/sha256:… (Range supported)
    C-->>P: streamed from store
```

Any node that holds a complete release serves it (seeding): C above can
fetch A's image from B if B replicated it, even if A is offline.

## Deletion and garbage collection

```mermaid
flowchart TD
    del["DELETE /v2/&lt;name&gt;/manifests/&lt;ref&gt;<br/>ocictl rm"] --> rr["remove release record<br/>(local only, not propagated)"]
    win["enforce_window"] --> rr
    gc["gc (periodic / ocictl gc)"] --> ret["retained = own ∪ pinned ∪ in-window<br/>∪ cache younger than grace"]
    ret --> rr
    rr --> sweep["sweep: unpin blob:&lt;sha256&gt; tags no<br/>retained release references, drop index entries"]
    sweep --> ib["iroh-blobs GC deletes unpinned data<br/>(blob_gc_interval_secs)"]
```

* Blobs are pinned in iroh-blobs under `blob:<sha256>` tags; in-flight
  downloads hold a temp tag so a sweep can't race them, and fetch vs. GC is
  serialised by an RwLock (fetch = read, GC/prune = write).
* `DELETE` on a blob is refused (405): only GC removes data.
* A deleted own release simply stops being announced; peers that already
  replicated it keep it until their own policy drops it.

## Wire protocols

| ALPN | purpose | encoding |
|---|---|---|
| `/iroh-gossip/1` | `Announcement::Release(ReleaseSummary)` on the publisher's topic; membership on the swarm topic | postcard |
| `ocid/sync/1` | `Hello{addr}`, `Inventory`, `GetRelease`, `ListTags` — one bi-stream per request; releases returned as their signed JSON bytes | postcard framing |
| `/iroh-bytes/…` (iroh-blobs) | blob download by BLAKE3 hash with incremental verification | bao |
| mDNS `_ocid._udp.local` | LAN discovery of other ocid endpoints (no tickets needed) | iroh-mdns-address-lookup |

Announcements are deliberately tiny (summary only) so they fit gossip limits;
the signed record is always fetched and verified via `sync`.

## Observability

`GET /metrics` on the registry port serves OpenMetrics text: `ocid_*`
(HTTP requests by method/route/status, bytes served/received, releases
published/replicated/failed, blobs fetched, announcements, sync requests by
type, mDNS discoveries, gossip topics, GC runs/removals, gauges for
neighbors/peers/releases) and iroh's endpoint and gossip metrics (`iroh_*`).

`GET /_ocid/events` serves a Server-Sent Events (SSE) stream of `DaemonEvent`s
(gossip announcements, releases saved, window prunes, peer connections, and HTTP
requests) consumed in real time by `ocitop`.

## Trust boundaries

* **Publisher** is trusted for *what* `name:tag` means (they sign it).
* **Providers** (any peer) are not trusted: BLAKE3 verifies every chunk while
  streaming, sha256 is re-checked after download against the signed record,
  and podman verifies sha256 again on pull.
* **Local registry** has no auth; it binds to loopback. Anyone who can reach
  it can push as you — same trust model as the local podman socket.
* **Deletion** is local. There is no signed tombstone; a publisher cannot
  recall a release from peers who already hold it.

## Testing

* `just test` — unit tests (policy parsing, windows, naming, records).
* `just e2e` — `scripts/e2e.sh` starts two daemons on the host and drives
  them with podman/curl/ocictl (and oras if present): publish, replicate,
  run a container from the replica, on-demand pull, Range/DELETE/metrics,
  follow/seed/pin windows with pruning and topic joins, aliases, GC, offline
  `ocictl`.

## Deferred

* delegation / multi-key publishers (Radicle "delegates"): a publisher is
  exactly one key
* signed tombstones (propagated deletion)
* registry authentication
