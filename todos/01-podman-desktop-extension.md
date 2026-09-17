# Podman Desktop Extension

> **Priority 01 · Tier 1 — adoption:** the distribution channel; GUI onboarding for the podman audience; lowest risk (API + SSE already exist). Tracked in [#15](https://github.com/safonas/ocid/issues/15).

## Context
Podman Desktop is the most direct adoption channel for the developer-collaboration
use case: a teammate pushes `localhost:5050/app:dev` and peers pull it over
QUIC without any registry. Today that workflow is invisible unless you know
about `ocictl`/`ocitop`. An extension surfaces the daemon, peers, and follows
inside the GUI users already have, at exactly the moment they are setting up
`podman` machines and registries.

## Proposal
- Publish a Podman Desktop extension (TypeScript, `podman-desktop` extension API)
  that bundles or manages the `ocid` daemon binary per platform (reuse the
  release assets / Homebrew formula).
- UI surfaces backed by the existing `/_ocid/*` JSON API and SSE stream:
  - daemon status, publisher id (`did:key`), connection ticket
    (copy/paste bootstrap, QR code for LAN nodes)
  - peers list + live SSE events (reusing the data `ocitop` renders)
  - releases table (`ocictl ls`) with follow/seed/pin controls editing
    `policy.toml` through the daemon
- Registry onboarding: a one-click "add local P2P registry" entry pointing
  podman at `127.0.0.1:5050`.
- Keep scope thin: the extension is a dashboard + onboarding, not a second
  implementation of the control plane.

## Why this priority
Lowest technical risk (HTTP API already exists; `ocitop` proves the event
stream), highest near-term user acquisition — it puts ocid in front of the
podman audience where the two-node demo is the pitch.
