# Cross-Publisher Blob Discovery & Layer Deduplication

> **Priority 09 · Tier 3 — scale:** big bandwidth win for base-layer-heavy fleets, but optimizes swarms that don't exist yet.

## Context
Blobs in `ocid` are content-addressed by BLAKE3 hash, so identical layers (e.g. common base images like `debian:bookworm` or `alpine:latest`) already share disk space. However, discovery across unrelated publishers is isolated: when pulling an image from Publisher B, the node only queries peers seeding Publisher B, even if connected peers seeding Publisher A already hold identical layer chunks.

## Proposal
- Maintain a cross-publisher blob provider index or distribute blob availability info during inventory exchanges.
- When downloading missing layer blobs, prioritize peers that already hold the matching BLAKE3/sha256 hash regardless of which publisher released the parent manifest.
- Significantly accelerates multi-tenant swarms and edge clusters where many custom images derive from identical base layers.
