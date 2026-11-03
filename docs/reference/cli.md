# CLI Reference

Two binaries: `ocid` (the daemon) and `ocictl` (the control CLI). Both take
`--home <dir>` / `$OCID_HOME` to pick the node state directory (default: XDG
data home, `~/.local/share/ocid`; a pre-existing legacy `~/.ocid` keeps being
used).

## `ocid` — the daemon

```sh
ocid [OPTIONS]
```

| Flag | Env | Description |
|------|-----|-------------|
| `--home <dir>` | `OCID_HOME` | Node home directory. |
| `--listen <addr>` | `OCID_LISTEN` | Registry/control/metrics listen address (default from `config.toml`, `127.0.0.1:5050`). |
| `--tls` | | Serve registry + control API over HTTPS with a self-signed CA (persisted into `config.toml`; CA in `$OCID_HOME/tls`). |
| `--peer <peer>` | | Bootstrap peer, repeatable: ticket, bare key (`did:key` or 64-hex), or DNS publisher name. |
| `--no-relay` | | Disable relay servers (direct/LAN connections only). |
| `--no-metrics` | | Do not expose `GET /metrics`. |

## `ocictl` — the control CLI

Global flag: `--home <dir>` / `$OCID_HOME` (same as `ocid`). Commands that talk
to the daemon need it running; `init`, `whoami`, `dns-record`, `policy`, `ls`
work offline.

### Node

| Command | Description |
|---------|-------------|
| `ocictl init` | Create a node identity and default configuration. |
| `ocictl whoami` | Print this node's identity (offline). |
| `ocictl status` | Daemon status: version, id, registry, uptime, policy summary. |
| `ocictl policy` | Print `policy.toml` (offline). |

### Peers and connecting

| Command | Description |
|---------|-------------|
| `ocictl ticket` | Print this node's connection ticket (daemon must run). |
| `ocictl peers` | List known peers. |
| `ocictl connect <peer>` | Add a peer: `peer` is a connection **ticket**, a bare key (`did:key:…` or 64-hex), or a **DNS publisher name**. Bare keys resolve via pkarr/DHT; DNS names via their signed record (with any `relay=`/`seed=` hints). |

`ocid --peer <peer>` and `POST /_ocid/peers {"peer": "…"}` accept the same
peer spec.

### Releases and replication

| Command | Description |
|---------|-------------|
| `ocictl ls` | List images held locally, with the rule that keeps each. |
| `ocictl pull <ref>` | Fetch a release from the swarm into the local store (as cache). |
| `ocictl publish [<ref>]` | (Re-)announce local releases — one reference or everything published by this node. |
| `ocictl seed <ref> [--latest\|--full\|--last N]` | Replicate an image and serve it to peers. `<pub>/<name>` keeps releases per mode (default `latest`); `<pub>/<name>:<tag>` keeps exactly that tag. |
| `ocictl unseed <ref>` | Stop seeding an image (all its rules, or one tag rule). |
| `ocictl follow <pub> [--latest\|--full\|--last N]` | Seed every image the publisher releases, per mode. |
| `ocictl unfollow <pub>` | Stop following a publisher. |
| `ocictl pin <ref>` | Pin a release: always kept and fetched, never garbage-collected. |
| `ocictl unpin <ref>` | Remove a pin. |
| `ocictl sync [<peer>]` | Replicate from peers now (all known peers, or one). |
| `ocictl rm <ref> [--all]` | Remove a release locally (not propagated). `<ref>` is `<pub>/<name>:<tag>` or an alias; without a tag, pass `--all`. |
| `ocictl gc [--dry-run] [--force]` | Garbage-collect: prune releases the policy does not want, delete unreferenced blobs. `--dry-run` reports only; `--force` ignores the grace period. |

### Aliases

| Command | Description |
|---------|-------------|
| `ocictl track <target> --as <alias>` | Alias a publisher or image: `localhost:5050/alice/<name>` / `localhost:5050/<alias>`. |
| `ocictl untrack <alias>` | Remove an alias. |

### DNS publisher names

| Command | Description |
|---------|-------------|
| `ocictl dns-record <zone> [--ts <unix>] [--relay <url>] [--seed <peer>]` | Print the signed `_ocid` TXT record binding `zone` to this node's key. `--relay`/`--seed` are optional signed reachability hints (never auto-filled); `--ts` overrides the timestamp (tests/tooling). |
| `ocictl resolve <zone>` | Resolve a DNS publisher name against the daemon: publisher key + pin state (pins on first sight). |
| `ocictl dns-unpin <zone>` | Drop the TOFU pin (deliberate key rotation); the next resolve re-pins. |

See [DNS Publisher Names](../dns-publisher-names.md) for the full workflow.

## See also

- [Configuration](config.md) — `config.toml` and `policy.toml`
- [HTTP API](api.md) — the `/_ocid` control API the CLI drives
- [ocitop](../guides/ocitop.md) — the TUI dashboard
