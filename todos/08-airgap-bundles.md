# Air-Gap Sneakernet Bundles (`ocictl export` / `import`)

> **Priority 08 · Tier 3 — scale:** makes the air-gapped pitch (defense/SCADA enclaves) real; small, self-contained feature.

## Context
Deploying containers into completely disconnected, air-gapped enclaves currently requires `podman save` / `podman load` tarballs, which lose Radicle-style provenance, signed release records, and incremental BLAKE3 blob deduplication.

## Proposal
- Implement `ocictl export <ref> --output bundle.ocid`:
  - Bundles the signed `Release` JSON, all referenced BLAKE3 blob files, and attached referrers (SBOMs, cosign/notation signatures) into a streaming archive.
- Implement `ocictl import bundle.ocid`:
  - Verifies publisher signatures and BLAKE3 chunk hashes upon ingest.
  - Adds release records to the local store index and immediately makes them servable via the local OCI registry and LAN mDNS to other enclave nodes.
