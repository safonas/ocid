# Podman Desktop Extension

A dashboard for the ocid daemon inside
[Podman Desktop](https://podman-desktop.io): status, releases, peers, live
events, connection ticket (copy + QR) and the policy actions — the same
`/_ocid` control API `ocictl` talks to. The extension is a thin view; every
action maps 1:1 to a daemon endpoint and the daemon stays the single policy
writer.

## What it does

- **Dashboard**: status header, releases table (seed / pin / follow /
  remove), peers + ticket connect, live event log, transfer progress strip
  (byte-accurate bars with throughput) whenever the daemon fetches from
  peers.
- **Setup card**: starts the daemon and makes podman trust it. The bundled
  daemon image (embedded in the extension artifact, per arch) is loaded and
  run as a pod serving HTTPS on `127.0.0.1:5050`, state bind-mounted at
  `~/.local/share/ocid`. When no bundle ships (PR builds, folder dev), it
  falls back to an `ocid` found on `PATH` (also with `--tls`).
- **TLS trust**: the daemon serves HTTPS; the extension trusts its
  self-signed CA and installs it into podman's `certs.d`
  (`~/.config/containers/certs.d/<host:port>/ca.crt`) — so push/pull verify
  the registry without `--tls-verify=false`. Plain-http daemons keep a
  `registries.conf` drop-in; setups that read neither fall back to a TLS
  bypass automatically.
- **Toasts** when a followed publisher ships a release, with a one-click
  *Pull* that runs `podman pull` as a Podman Desktop task.
- **Auto-pull** (opt-in, default off): new releases from followed peers are
  pulled into podman automatically. Persisted as the `ocid.autoPull`
  setting.
- **Push image to ocid peers** on the Images page context menu: pushes to
  the daemon's registry — the daemon signs and announces it, and peers
  following you replicate it automatically.
- **Daemon stop button + keep-alive**: the pod's lifetime is the
  extension's by default (it stops on disable/removal); the
  `ocid.keepDaemonAlive` setting opts into a daemon that outlives it
  (systemd quadlet on Linux, `podman run --restart=always` elsewhere).

Settings: `ocid.registryUrl` (default `https://127.0.0.1:5050`), the
extension talks to the daemon on the host's loopback.

## Status

Phase 3 (bundled TLS daemon as a pod) is in progress
([#15](https://github.com/safonas/ocid/issues/15)). Supported: Linux with a
rootless podman connection. Known gaps: macOS runs the pod untested (the
PATH-daemon fallback works), and the bundled image is only produced by the
release workflow — PR/folder builds fall back to a PATH daemon.

## Install

1. In Podman Desktop: **Settings → Extensions → Install from OCI image** →
   `ghcr.io/safonas/ocid-extension:testing` (versioned tags track releases).
2. Open the ocid dashboard and click **Start ocid daemon**: the bundled
   image is loaded and run as a pod (TLS on `127.0.0.1:5050`, trusted by
   podman via certs.d). No separate daemon install needed.
3. To try the peer flow, repeat on a second machine (or a second
   `OCID_HOME`) and paste one node's ticket into the other's dashboard.

## Develop

```sh
just ext-build          # build backend (dist/) + frontend (media/)
just ext-check         # tsc + svelte-check
just ext-test          # unit tests for extension internals (no daemon)
just ext-smoke         # run the extension client against a throwaway daemon
just ext-image         # build the OCI artifact (for the catalog)
just ext-folder        # assemble a folder for PD's local-folder install
```

The daemon must bind the **host's** 127.0.0.1 — `just run` does that (state
under `./.dev/ocid-home`). Load the folder in Podman Desktop via
**Settings → Extensions → Add a local folder extension…**; a second local
node is `OCID_HOME=.dev/node-b just run --listen 127.0.0.1:15061`.

See the extension's
[README](https://github.com/safonas/ocid/blob/main/extensions/podman-desktop/README.md)
for design notes (backend owns all network I/O; the webview is a pure view)
and the publishing flow.

## See also

- [HTTP API](../reference/api.md) — the control API behind every action
- [Development](../development.md) — the `just ext-*` recipes
