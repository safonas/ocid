# ocitop

`ocitop` is the terminal dashboard for the daemon: live status, releases,
peers, and the daemon's event stream over the same `/_ocid` API the CLI and
the Podman Desktop extension use.

```sh
ocitop              # daemon on the default registry (127.0.0.1:5050)
ocitop --home <dir> # / $OCID_HOME: node whose daemon to watch
```

## Tabs

| Key | Tab | Shows |
|-----|-----|-------|
| `1` | **Images** | Releases held locally: publisher, name, tag, size, completeness, and the policy rule that keeps each one. |
| `2` | **Peers** | Known peers: neighbor/known state, last seen. |
| `3` | **Events** | The full daemon event log (SSE stream, reconnects automatically). |

`Tab` cycles tabs; the bottom ticker collapses on the Events tab because
the tab *is* the log.

## Keys

| Key | Action |
|-----|--------|
| `1` / `2` / `3` | Switch tab |
| `Tab` | Next tab |
| `↓` / `j`, `↑` / `k` | Move selection |
| `r` | Refresh now (polls every 2s regardless) |
| `y` | `sync` — replicate from peers now |
| `g` | GC (asks for confirmation) |
| `G` | GC ignoring the grace period (asks for confirmation) |
| `s` | Seed the selected release — prompts for the retention mode (default `latest`) |
| `p` | Pin / unpin the selected release |
| `f` | Follow the selected release's publisher |
| `d` | Remove the selected release (asks for confirmation) |
| `q` or `Ctrl-C` | Quit |

Actions apply to the selected release; your own releases are always kept,
so policy actions only target foreign ones.

## See also

- [CLI Reference](../reference/cli.md) — the same actions from the shell
- [HTTP API](../reference/api.md) — what the dashboard polls and subscribes to
