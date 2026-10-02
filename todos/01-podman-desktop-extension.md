# Podman Desktop Extension

> **Priority 01 · Tier 1 — adoption:** the distribution channel; GUI onboarding for the podman audience; lowest risk (API + SSE already exist). Tracked in [#15](https://github.com/safonas/ocid/issues/15).

## Status

**Phase 1 (dashboard) shipped** — #24 (policy mutation endpoints), #25
(wolfi toolchain), #26 (the extension). What exists in
`extensions/podman-desktop`:

- backend owns all network I/O: typed `/_ocid` client, 2s poller (ocitop
  model), SSE watcher with ring buffer; the webview is a pure view
- releases table with seed/unseed/pin/unpin/follow/remove via the
  `/_ocid/policy/*` endpoints, peers + ticket connect, live events log,
  ticket copy + QR
- `@podman-desktop/ui-svelte` + Tailwind + `--pd-*` theme variables
  (current Podman Desktop conventions); `src/types.ts` mirrors
  `ocid-core`'s API DTOs
- scratch OCI artifact for the catalog; `just ext-*` recipes
  (node toolchain containerized like cargo)

**Phase 2 (testing round, Linux)** — setup integrations:

- **Registry registration**: setup card + `src/registries.ts` write the
  user-level `registries.conf.d/100-ocid.conf` drop-in (`insecure = true`
  for the loopback registry); push/pull drop the hardcoded
  `--tls-verify=false` and only fall back to it when unregistered or after
  a plain failure (rootful podman, podman machines).
- **Daemon lifecycle v0**: detect `ocid` on `PATH` + common install dirs
  (cached, 30s negative retry); setup card starts it detached (log in the
  extension's `storagePath`) or shows the install one-liner.
- **Distribution**: `extension` workflow builds the multi-arch artifact and
  pushes `ghcr.io/safonas/ocid-extension:<tag>` + `:testing` on every
  published release; `just cut-release` bumps the extension's package.json
  in lockstep with Cargo.toml.

## Phase 3 — TLS + self-contained daemon (in progress, targets v0.7.0)

Replace the "install the daemon yourself, then register an insecure http
registry" flow with a trusted, bundled, self-supervised daemon.

### Daemon TLS (`ocid` + `ocid-core`) — PR 1
- [ ] `config.toml`: `tls = "off" | "auto"` (default `off`, so dev/e2e are
  unchanged); `--tls` flag persisted like `--listen`.
- [ ] First boot with `auto`: rcgen self-signed CA + server cert under
  `OCID_HOME/tls/` (SANs `localhost`, `127.0.0.1`, `::1`,
  `host.containers.internal`); `server.key`/`ca.key` mode 0600; regenerated
  wholesale if any file is missing.
- [ ] rustls listener for `/v2` + `/_ocid` (axum-server,
  `tls-rustls-no-provider` + explicit ring provider install); `/metrics`
  unchanged.
- [ ] `ocictl`/`ocitop` dial `https` and trust `tls/ca.crt` when `tls =
  "auto"` (`Client::from_config`).
- [ ] Cosmetic: stop emitting `DaemonEvent::HttpRequest` for `GET /_ocid/*`
  (the 2s status/peers/releases poll drowns out the event log).

### Extension — PR 2
- [ ] **TLS trust**: install `tls/ca.crt` into
  `~/.config/containers/certs.d/localhost:5050/ca.crt` (Linux rootless);
  macOS via `podman machine ssh`; drop `--tls-verify=false` where trust is
  installed (keep the registries.conf drop-in + bypass for `tls = "off"`
  and rootful/machines).
- [ ] **Daemon delivery**: bundle per-platform `ocid` binaries in the
  extension artifact (cross-compile via zigbuild); no native Windows daemon
  (WSL follow-up).
- [ ] **Daemon as a pod**: build `localhost/ocid-daemon:ext` from the
  bundled binary at install; run via quadlet drop-in
  (`~/.config/containers/systemd/ocid.container`, host-networked Linux)
  with `podman run --restart=always` fallback in machine VMs; `OCID_HOME`
  bind-mounted at `~/.local/share/ocid` (`:Z`); `PATH` daemon stays the
  fallback.
- [ ] **Auto-pull**: `ocid.autoPull` checkbox in the onboarding card
  (default off) — on a followed `release_saved` event, pull into podman
  automatically instead of showing the manual Pull toast.

## Remaining

- **macOS**: verify the pod + `machine ssh` trust install on a real machine
  (best-effort until then; QUIC is relay-only behind the machine NAT).
- **Catalog submission**: PR to podman-desktop-catalog once feedback
  stabilizes.
- Demo material: see [todos/14-website-demo.md](14-website-demo.md)
  (GitHub Pages + asciinema) — deliberately deferred.

## Context
Podman Desktop is the most direct adoption channel for the developer-collaboration
use case: a teammate pushes `localhost:5050/app:dev` and peers pull it over
QUIC without any registry. Today that workflow is invisible unless you know
about `ocictl`/`ocitop`. An extension surfaces the daemon, peers, and follows
inside the GUI users already have, at exactly the moment they are setting up
`podman` machines and registries.

## Proposal
A dashboard + onboarding surface, not a second implementation of the control
plane: every capability maps 1:1 to a `/_ocid` endpoint, the daemon stays the
single policy writer. Architecture and rules are documented in
`docs/DESIGN.md` ("Podman Desktop extension"); development and local-testing
instructions in `extensions/podman-desktop/README.md`.

## Why this priority
Lowest technical risk (HTTP API already exists; `ocitop` proves the event
stream), highest near-term user acquisition — it puts ocid in front of the
podman audience where the two-node demo is the pitch.
