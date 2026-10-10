# DNS Publisher Names

Human-readable domain names instead of 64-character hex publisher ids.
A domain like `images.example.com` can stand in for your publisher key
everywhere a `<publisher>` appears: registry paths, `ocictl pull`,
`ocictl follow`, `ocictl seed`, `ocictl pin`, and `policy.toml`.

```sh
# instead of:
podman pull --tls-verify=false 127.0.0.1:5050/3f9a…c1/app:1.0

# you can say:
podman pull --tls-verify=false 127.0.0.1:5050/images.example.com/app:1.0
```

The domain takes the same first-path-segment slot as `<hex>` — the
loopback-registry trick is unchanged; only name resolution grows a branch.

## How it works

A DNS zone claims a human name for a publisher key via a **signed TXT
record** at `_ocid.<zone>`:

```
_oid.images.example.com. 300 IN TXT "v=ocid1 k=<64-hex> ts=<unix> sig=<128-hex>"
```

The signature is made by the publisher key itself over the canonical
payload `ocid1 <zone> <k> <ts>`. This proves **mutual consent**: the zone
admin points the name at the key, and only the key's owner could have
signed the record. A domain cannot be saddled with a key, and a key
cannot claim a domain it wasn't published under.

Since every release is already signed by the publisher key, the DNS
record only has to convince you **which key owns the name** — everything
published under it verifies itself.

### Trust model

Plain DNS is spoofable, so trust is anchored locally:

| Mechanism | What it does |
|---|---|
| **Signature verification** | Every record is verified against the key it carries. Always. |
| **Zone binding** | The zone name is part of the signed bytes — a record cannot be replayed across zones. |
| **Freshness window** | Records older than `dns_max_age_secs` (default 30 days) are rejected. Limits replay of stale records. |
| **TOFU pinning** | The first verified resolution pins the key in `$OCID_HOME/dns-pins.json`. A later record for the same zone with a different key is rejected as a possible hijack. |
| **Rotation escape hatch** | `ocictl dns-unpin <zone>` drops the pin; the next resolve re-pins whatever the zone publishes. |
| **Caching** | Positive and negative lookups are cached briefly (TTL-capped). Errors are never cached — every pull re-checks a bad record. |
| **Fall-through** | A domain with no record (or a dotted self-name like `my.app`) falls through to the existing alias/self name rules. DNS adds a branch; it never breaks existing names. |

DNS is an **untrusted lookup**; the pins are the root.

## For publishers: claim your name

Generate the signed record (fully offline — signs with your local identity):

```sh
ocictl dns-record images.example.com
```

Output:

```
Publish this record in the images.example.com zone:

  _ocid.images.example.com. 300 IN TXT "v=ocid1 k=<your-hex> ts=1791503161 sig=<128-hex>"

Once it propagates, pulls use the name instead of hex:

  podman pull http://127.0.0.1:5050/images.example.com/<name>:<tag>
```

Publish that TXT line in your DNS zone (BIND, Cloudflare, etc.). No
ocid infrastructure is involved — any DNS provider works.

To refresh the timestamp before it ages out (records older than 30 days
are rejected), re-run `ocictl dns-record` and re-publish.

### Zone requirements

The zone must be a syntactically valid DNS name:

- At least one dot (`images.example.com`, not `buildah`)
- Lowercase hostname charset: `a-z`, `0-9`, `-`, `.`
- No leading/trailing hyphens in labels
- No all-numeric labels (so IP literals like `1.2.3.4` are not treated as zones)
- No trailing dot
- Labels 1–63 chars, total ≤ 253 chars

## For consumers: pull, follow, and seed by name

### Pull by domain

```sh
podman pull --tls-verify=false 127.0.0.1:5050/images.example.com/app:1.0
```

The daemon resolves `images.example.com` → publisher key via DNS,
verifies the signature, pins it (TOFU first sight), and fetches the
release from that publisher on demand.

### Inspect a name

Needs the daemon running (it performs the DNS lookup):

```sh
ocictl resolve images.example.com
```

```
zone       images.example.com
publisher  3f9a…c1
state      pinned (verified, fresh, TOFU-pinned)

pulls via:  podman pull <registry>/images.example.com/<name>:<tag>
```

States: `pinned` (existing pin confirmed), `new` (pinned now),
`no-record` (falls through to name rules), `disabled` (`dns = "off"`),
`error` (stale record, pin mismatch, bad signature — with detail).

### Policy in domain form

Policy commands need the daemon running (it resolves and pins the domain
first). Offline, use the publisher id or an alias instead:

```sh
ocictl follow images.example.com          # every image they publish
ocictl seed images.example.com/app:1.0    # exactly this tag
ocictl pin images.example.com/app:1.0     # always kept, never GC'd
```

`policy.toml` keeps the human-readable domain form and records the
resolved key in a `[dns]` mapping table, so policy evaluation never
needs DNS at load time:

```toml
[[follow]]
publisher = "images.example.com"

[[seed]]
ref  = "images.example.com/app:1.0"

[dns]
images.example.com = "3f9a…c1"
```

### Key rotation (deliberate)

If a publisher legitimately changes keys (lost key, org change). Both
commands need the daemon running:

```sh
ocictl dns-unpin images.example.com     # drop the pin
ocictl resolve images.example.com       # re-pins the new key
```

If you did **not** expect a key change, do not unpin — treat it as a
hijack attempt and investigate.

## Configuration (`config.toml`)

```toml
dns = "on"                        # default: on. "off" disables all DNS lookups.
dns_nameserver = "10.0.0.53"      # optional: custom resolver (IP literal, port defaults to 53).
                                  # default: system resolver. Useful for air-gapped setups.
dns_max_age_secs = 2592000        # default: 30 days. Records older than this are rejected.
```

## HTTP API

The daemon exposes the same operations for tooling (e.g. the Podman
Desktop extension):

| Method | Path | Description |
|---|---|---|
| `GET` | `/_ocid/dns/resolve?zone=<zone>` | Resolve a zone. Never fails with HTTP error — the `state` field carries the outcome. |
| `POST` | `/_ocid/dns/unpin` | Drop the TOFU pin. Body: `{"zone": "<zone>"}`. |

Example:

```sh
curl -s "http://127.0.0.1:5050/_ocid/dns/resolve?zone=images.example.com" | jq .
```

```json
{
  "zone": "images.example.com",
  "state": "pinned",
  "publisher": "3f9a…c1",
  "detail": null
}
```

## Files on disk

| Path | Purpose |
|---|---|
| `$OCID_HOME/dns-pins.json` | TOFU pin store: zone → pinned publisher key + `pinned_at` timestamp. Atomic writes. |
| `$OCID_HOME/policy.toml` `[dns]` table | Zone → resolved key mapping, written by the daemon when a domain-form policy rule is saved. |

## Resolution precedence

When the registry sees `/v2/<first-segment>/<rest>/…`, it tries in order:

1. **64-hex** → explicit publisher
2. **Domain with verified `_ocid` record** → publisher from the record (TOFU-pinned)
3. **Image alias** (whole repo matches an alias)
4. **Publisher alias** (first segment matches an alias)
5. **Implicit self** → your own publisher id

A domain that has no record (or DNS is disabled) falls through to steps
3–5 unchanged.

## Security considerations

- **Hijack detection:** if a pinned zone suddenly resolves to a different
  key, resolution fails loudly. The error message names both keys.
- **Stale records:** a record signed more than `dns_max_age_secs` ago is
  rejected even if the signature is valid — forces periodic re-publishing.
- **No DNSSEC required:** the signature is the security boundary, not
  DNSSEC. (Opportunistic DNSSEC validation may come later.)
- **Offline after first sight:** once pinned, the name resolves from the
  pin store. First contact always requires DNS.
- **Pin store is local:** `$OCID_HOME/dns-pins.json` is per-machine.
  A fresh node pins on first verified resolution — there is no
  cross-node pin distribution (by design; pins are a local trust root).

## Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `state: error` — "refusing to switch keys" | Zone now points to a different key than the pin | If intentional: `ocictl dns-unpin <zone>`. If not: possible hijack — investigate. |
| `state: error` — "is stale" | Record older than `dns_max_age_secs` | Publisher re-runs `ocictl dns-record` and re-publishes. |
| `state: no-record` | No `_ocid` TXT at the zone | Check `dig TXT _ocid.<zone>`; publish the record. |
| `state: disabled` | `dns = "off"` in config.toml | Set `dns = "on"` and restart. |
| Pull by domain → `MANIFEST_UNKNOWN` | Name resolved but the publisher has no such image | Check the image name/tag with the publisher. |
| Pull by domain → `NAME_INVALID` | DNS record failed verification | See `ocictl resolve <zone>` for the detail. |

## See also

- [README.md](../README.md) — naming table, CLI reference
- [DESIGN.md](DESIGN.md) — architecture, trust boundaries, resolution diagram
- [todos/02-dns-publisher-records.md](../todos/02-dns-publisher-records.md) — design notes, roadmap
