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

## Remaining (after the testing round)

- **macOS registration**: `podman machine ssh` + `host.containers.internal`
  drop-in inside the VM (the host drop-in does not reach it); needs testing
  on a real machine.
- **Daemon bundling**: ship per-platform `ocid` binaries inside the
  extension artifact (decided: no native Windows daemon — WSL follow-up).
- **Catalog submission**: PR to podman-desktop-catalog once feedback
  stabilizes.
- Demo material: see [todos/14-website-demo.md](14-website-demo.md)
  (GitHub Pages + asciinema) — deliberately not part of the testing round.

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
