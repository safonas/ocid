# Live Swarm Monitor TUI (`ocitop`)

## Status
Implemented in `crates/ocitop` as a dedicated TUI binary (`ocitop`).
Uses Ratatui and crossterm, polling the daemon control API and streaming real-time events over SSE (`/_ocid/events`).

## Features Implemented
- Status header with node DID, active peers, releases count, policy rules, and uptime.
- Images tab with table of local releases, retention status, verification check, and inspector detail pane.
- Peers tab with endpoint IDs, DIDs, neighbor status, and last-seen timing.
- Live Events tab & ticker streaming SSE events (`DaemonEvent`: gossip, release saved, prune, peer change, HTTP requests).
- Interactive policy and lifecycle actions: seed, follow, pin, sync, GC, and delete with confirmation dialogs.

## Remaining Enhancements
- Real-time active blob transfers with progress bars and speeds.
- Ping latency and QUIC connection quality metrics per peer.
