# HTTP API

The daemon serves three HTTP surfaces on `listen` (default
`127.0.0.1:5050`, `https://` when `tls = "auto"`):

- `/_ocid/*` — the control API `ocictl`, `ocitop` and the Podman Desktop
  extension talk to. No authentication: it binds to loopback, same trust
  model as the local podman socket.
- `/v2/*` — the embedded OCI v2 registry `podman`/`docker`/`oras` use.
- `/metrics` — Prometheus/OpenMetrics (unless `metrics = false`).

Errors are `{"error": "…"}` with a 4xx/5xx status.

## Status

`GET /_ocid/status` → `Status`

```json
{
  "id": "<64-hex>",
  "did": "did:key:z6Mk…",
  "ticket": "<connection ticket>",
  "registry": "http://127.0.0.1:5050",
  "uptime_secs": 3600,
  "neighbors": ["<EndpointId>"],
  "known_peers": 3,
  "releases": 7,
  "seeds": ["3f9a…c1/app"],
  "follows": ["3f9a…c1"],
  "pins": ["3f9a…c1/app:1.0"],
  "pkarr": "on",
  "version": "0.9.0"
}
```

## Events

`GET /_ocid/events` — Server-Sent Events; one JSON `DaemonEvent` per `data:`
line. Event types (tagged `type`):

| Type | Fields | Meaning |
|------|--------|---------|
| `gossip` | `publisher`, `name`, `tag`, `outbound` | Release announcement received/broadcast. |
| `pruned` | `publisher`, `name`, `tag`, `reason` | Release pruned or kept by window enforcement. |
| `release_saved` | `publisher`, `name`, `tag`, `blobs` | Release replicated or published. |
| `pull_progress` | `publisher`, `name`, `tag`, `blobs_done`, `blobs_total`, `bytes_done`, `bytes_total` | Incremental fetch progress. |
| `fetch_failed` | `publisher`, `name`, `tag`, `error` | Fetch failed. |
| `peer_change` | `id`, `connected` | Peer connected/disconnected. |
| `http_request` | `method`, `path`, `status` | Registry/control request processed. |

## Peers

- `GET /_ocid/peers` → array of `PeerInfo` (`id`, `neighbor`, `known`,
  `last_seen_secs`).
- `POST /_ocid/peers` — body `{"peer": "<ticket | did:key | 64-hex | DNS name>"}`
  (the `ticket` field name stays accepted as an alias) → `{"id": "<EndpointId>"}`.

## Releases

`GET /_ocid/releases` → array of `ReleaseInfo`: the release summary
(publisher, name, tag, digests, timestamp, …) plus `complete`, `size`,
`blobs`, `mine`.

## Fetching and maintenance

| Method + path | Body | Response |
|---|---|---|
| `POST /_ocid/pull` | `{"reference": "<pub>/<name>[:<tag>]"}` | `{"ok": true}` |
| `POST /_ocid/announce` | `{"reference": "<ref>"}` (optional — empty announces everything of yours) | `{"announced": N}` |
| `POST /_ocid/sync` | `{"peer": "<peer>"}` (optional — all known peers) | `{"synced": N, "failed": ["…"]}` |
| `POST /_ocid/gc` | `{"dry_run": false, "force": false}` (both optional) | `GcReport`: `dry_run`, `releases_removed`, `blobs_removed`, `bytes_freed`, `uploads_removed` |
| `POST /_ocid/rm` | `{"reference": "<ref>", "all_tags": false}` | `{"removed": ["…"], "still_wanted": false}` |

`still_wanted` true means the policy still wants the image — it will be
replicated again unless unseeded.

## Policy mutations

All return `PolicyChangeResp`: `{"changed": bool, "reference": "<canonical rule>"}`.
`mode` is `latest` (default), `last:N`, or `full`.

| Method + path | Body |
|---|---|
| `POST /_ocid/policy/reload` | — (re-read `policy.toml` from disk) |
| `POST /_ocid/policy/seed` | `{"reference": "<pub>/<name>[:<tag>]", "mode": "latest"}` |
| `POST /_ocid/policy/unseed` | `{"reference": "…"}` |
| `POST /_ocid/policy/follow` | `{"publisher": "…", "mode": "latest"}` |
| `POST /_ocid/policy/unfollow` | `{"publisher": "…"}` |
| `POST /_ocid/policy/pin` | `{"reference": "<pub>/<name>:<tag>"}` |
| `POST /_ocid/policy/unpin` | `{"reference": "…"}` |

Publishers and refs accept the DNS domain form (`images.example.com`); the
daemon resolves and pins the name first and records the key in
`policy.toml`'s `[dns]` table.

## DNS publisher names

- `GET /_ocid/dns/resolve?zone=<zone>` → `DnsResolveResp`:

```json
{
  "zone": "images.example.com",
  "state": "pinned",
  "publisher": "<64-hex>",
  "relay": "https://use1-1.relay.iroh.network",
  "seed": "<64-hex>",
  "detail": null
}
```

  `state`: `pinned` | `new` | `no-record` | `disabled` | `error` (with
  `detail`). Never fails with an HTTP error. `relay`/`seed` appear only when
  the record carries signed hints.

- `POST /_ocid/dns/unpin` — body `{"zone": "<zone>"}` → `{"removed": true}`.

See [DNS Publisher Names](../dns-publisher-names.md).

## Metrics

`GET /metrics` — OpenMetrics (Prometheus) format, served on `listen` when
`metrics = true`.

## OCI registry (`/v2`)

The embedded registry implements the OCI Distribution Spec v2 with 1.1
referrers: `GET /v2`, blob upload (`POST`/`PATCH`/`PUT`/`DELETE`), manifest
`GET`/`PUT`, tags and catalog, referrers API, blob `GET` with Range support.
Repository paths carry the publisher segment (`/v2/<publisher>/<name>/…`).

### Delete manifest

`DELETE /v2/<name>/manifests/<ref>` — removes the **local** release record
only; deletion is never propagated to peers (no signed tombstones). `ocictl
rm` and window enforcement do the same thing. Pushes to other publishers'
namespaces are refused with `403`. Blob `DELETE` is refused with `405` —
only GC removes blobs, after no retained release references them.

See [Trust Model](../architecture/trust-model.md#deletion-trust) and
[Workflows](../architecture/workflows.md#garbage-collection).
