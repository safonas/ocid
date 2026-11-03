# Configuration

Two TOML files live in the node home (`$OCID_HOME`, default
`~/.local/share/ocid`; a pre-existing legacy `~/.ocid` keeps being used):

- `config.toml` — daemon configuration
- `policy.toml` — the replication/retention policy

## `config.toml`

Values shown are the defaults:

```toml
listen = "127.0.0.1:5050"     # registry + control API + metrics
tls = "off"                   # "off" (plain http) | "auto" (self-signed CA under $OCID_HOME/tls)
relay = "default"             # "default" (n0 relays) | "disabled"
pkarr = "on"                  # publish/resolve endpoint addresses via pkarr/mainline DHT
p2p_port = 0                  # fixed UDP port for iroh (0 = random)
mdns = true                   # discover other ocid nodes on the LAN
fetch_timeout_secs = 120      # max wait for an on-demand p2p fetch behind a registry request
metrics = true                # expose Prometheus/OpenMetrics at GET /metrics
gc_interval_secs = 3600        # automatic GC cadence (0 = manual `ocictl gc` only)
gc_grace_secs = 86400         # how long replicated releases outside the policy are kept after fetch
blob_gc_interval_secs = 60    # how often the blob store deletes unpinned data
dns = "on"                    # resolve `<domain>/<name>` refs via _ocid TXT records
# dns_nameserver = "10.0.0.1:53"  # custom resolver (default: system resolver)
dns_max_age_secs = 2592000    # reject _ocid DNS records signed more than this long ago
```

| Key | Type | Description |
|-----|------|-------------|
| `listen` | socket addr | Where the OCI registry, `/_ocid` control API and `/metrics` listen. The daemon **persists** a `--listen` override into this file (so `ocictl` finds it). |
| `tls` | `"off"` \| `"auto"` | `"auto"` serves HTTPS with a self-signed CA generated under `$OCID_HOME/tls` — transport encryption and a CA clients can pin, not authentication. `ocid --tls` persists `auto`. |
| `relay` | `"default"` \| `"disabled"` | iroh relay servers for NAT traversal. With `"disabled"`, a node still reaches LAN peers (mDNS) and anyone whose ticket or DNS record carries a usable hint, but cannot dial through relays — bare-key (pkarr) connects need `"default"`. |
| `pkarr` | `"on"` \| `"off"` | `"off"` removes the pkarr publisher/resolver from the endpoint: no address publishing into the mainline DHT, no lookup — bare-key connects then need tickets, mDNS, or signed DNS hints. |
| `p2p_port` | port | Fixed UDP port for the iroh endpoint (`0` = random). |
| `mdns` | bool | Discover peers on the LAN via `_ocid._udp` mDNS. |
| `fetch_timeout_secs` | seconds | Max seconds a registry request waits for an on-demand p2p fetch. |
| `metrics` | bool | Expose `GET /metrics` on `listen` (OpenMetrics). |
| `gc_interval_secs` | seconds | Run GC every N seconds (`0` = manual only). |
| `gc_grace_secs` | seconds | Replicated releases outside the policy are kept at least this long after fetch before automatic GC prunes them. |
| `blob_gc_interval_secs` | seconds | How often the blob store deletes unpinned data. |
| `dns` | `"on"` \| `"off"` | Domain-form repository segments (`images.example.com/app`) resolve via `_ocid` TXT records. `"off"` makes domains fall through to the alias/self rules. |
| `dns_nameserver` | addr | Custom DNS resolver (`host[:port]`) — the system resolver by default. Useful for air-gapped setups. |
| `dns_max_age_secs` | seconds | Records signed more than this ago are rejected (forces periodic re-publishing). |

See [DNS Publisher Names](../dns-publisher-names.md) for the DNS trust model.

## `policy.toml`

The policy is the heart of the Radicle-style replication model:

```toml
# image refs we replicate and serve to others
[[seed]]
ref = "3f9a…c1/app"          # <publisher>/<name>, kept per mode
mode = "full"

[[seed]]
ref = "3f9a…c1/app:1.0"      # <publisher>/<name>:<tag> keeps exactly that tag

# publishers whose every image we replicate, per mode
[[follow]]
publisher = "3f9a…c1"
mode = "last:3"

# <publisher>/<name>:<tag> releases that are always kept, never GC'd
pin = ["3f9a…c1/app:1.0"]

# human-friendly local names
[alias]
alice = "3f9a…c1"            # -> localhost:5050/alice/<name>
team-app = "3f9a…c1/app"     # -> localhost:5050/team-app

# zone -> resolved publisher key, written by the daemon when a domain-form
# policy rule is saved; evaluation never needs DNS at load time
[dns]
images.example.com = "3f9a…c1"
```

- **`seed`** — image refs to replicate and serve. `<publisher>/<name>` keeps
  releases per `mode`; `<publisher>/<name>:<tag>` keeps exactly that tag.
- **`follow`** — publishers whose every image we replicate, per `mode`.
- **`pin`** — `<publisher>/<name>:<tag>` refs always kept and fetched.
- **`alias`** — local names for a publisher or an image (registry paths keep
  working after renames; `ocictl track` edits this).
- **`dns`** — resolved keys for domain-form rules (`follow
  images.example.com`); written by the daemon, not by hand.

### Modes

| Mode | Keeps |
|------|-------|
| `latest` | the newest release (default; same as `last:1`) |
| `last:N` | the N newest releases |
| `full` | every release |

Multiple rules for one image combine into the widest window; pins and your
own releases are never pruned. `ocictl seed/follow/unseed/…` (and the
extension/TUI) mutate this file through the daemon — it stays the single
policy writer.

## See also

- [CLI Reference](cli.md) — commands that edit these files
- [Data Model](../architecture/data-model.md) — what ends up on disk
- [Workflows](../architecture/workflows.md) — how the policy drives replication
