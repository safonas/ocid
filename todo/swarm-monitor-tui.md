# Live Swarm Monitor TUI (`ocictl top`)

## Context
Operators currently monitor daemon activity through structured logs or Prometheus metrics (`GET /metrics`). There is no interactive real-time visualization of swarm health and transfers.

## Proposal
- Implement an interactive terminal UI command: `ocictl top` (or `ocictl monitor`).
- Features:
  - Live list of connected peers, ping latency, and QUIC connection quality.
  - Real-time active blob transfers with progress bars, speeds, and provider endpoints.
  - Live stream of gossip announcements across joined publisher topics.
  - Cache size, active pins, and retention window status.
