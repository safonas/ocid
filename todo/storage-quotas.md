# Configurable Storage Quotas & LRU Eviction

## Context
Garbage collection currently relies on time-based retention windows and grace periods (`gc_interval_secs`, `gc_grace_secs`). Edge devices with tight storage constraints can risk running out of disk space if multiple large unseeded images are pulled before the grace period expires.

## Proposal
- Add storage quota configurations in `config.toml`:
  ```toml
  max_storage_bytes = "20GB"
  high_watermark_pct = 90
  low_watermark_pct = 75
  ```
- Implement an LRU (Least Recently Used) cache eviction policy:
  - Track last access time for cached releases and unpinned blobs.
  - When disk usage crosses `high_watermark_pct`, trigger automatic background eviction of unpinned cache down to `low_watermark_pct`.
  - Pinned releases and actively seeded windows are strictly protected from eviction.
