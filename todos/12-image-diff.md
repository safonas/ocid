# Image Release Diffing (`ocictl diff`)

> **Priority 12 · Backlog:** demo appeal ("diff before you pull"); moderate effort, no dependencies.

## Context
Operators and developers replicating images need visibility into what changes between release tags or between remote announcements before fetching gigabytes of layer data.

## Proposal
- Implement `ocictl diff <ref1> <ref2>`:
  - Compares manifests, layer hashes, configurations (entrypoint, env, labels).
  - Highlights added/removed layers and overall size delta.
  - Compares attached referrers (e.g. SBOM package diff, new signatures, attestation changes).
- Works both locally against indexed releases and remotely by querying candidate peers for manifest references without downloading all blob layers.
