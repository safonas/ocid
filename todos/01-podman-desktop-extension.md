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

## Remaining (Phase 2)

- **Registry registration** (researched, see #15 comment): PD's settings
  dialog is an `auth.json` login flow (https + credentials) and cannot add
  an anonymous http registry — the extension must write a `registries.conf`
  drop-in itself: `~/.config/containers/registries.conf.d/` on Linux
  (rootless), `podman machine ssh` + `host.containers.internal:5050` on
  macOS. `registry.suggestRegistry()` rejected (funnels into the login
  dialog). Est. 2–3h Linux (verifiable: push works without
  `--tls-verify=false`), +1–2h macOS (needs a real machine to test).
- **Daemon delivery**: bundle per-platform `ocid` binaries in the extension
  artifact (decided; no native Windows daemon — WSL follow-up).
- **Daemon lifecycle**: detect/start/supervise the bundled daemon.
- Demo GIF of the two-node flow drivable from the GUI, for the README.

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
