// Types mirroring ocid-core/src/api.rs — the daemon's /_ocid/ control API.
// Keep in sync with the Rust DTOs; the extension never invents shapes.

export interface Status {
  id: string;
  did: string;
  ticket: string;
  registry: string;
  uptime_secs: number;
  neighbors: string[];
  known_peers: number;
  releases: number;
  seeds: string[];
  follows: string[];
  pins: string[];
  version: string;
}

export interface PeerInfo {
  id: string;
  neighbor: boolean;
  known: boolean;
  last_seen_secs?: number | null;
}

export interface ReleaseInfo {
  // ReleaseSummary, flattened by serde
  publisher: string;
  name: string;
  tag: string;
  manifest_digest: string;
  timestamp: number;
  complete: boolean;
  size: number;
  blobs: number;
  mine: boolean;
}

export type DaemonEvent =
  | { type: 'gossip'; publisher: string; name: string; tag: string; outbound: boolean }
  | { type: 'pruned'; publisher: string; name: string; tag: string; reason: string }
  | { type: 'release_saved'; publisher: string; name: string; tag: string; blobs: number }
  | { type: 'peer_change'; id: string; connected: boolean }
  | { type: 'http_request'; method: string; path: string; status: number };

export interface PolicyChangeResp {
  changed: boolean;
  reference: string;
}

export interface SyncResp {
  synced: number;
  failed: string[];
}

export interface GcReport {
  dry_run: boolean;
  releases_removed: string[];
  blobs_removed: number;
  bytes_freed: number;
  uploads_removed: number;
}

export interface RmResp {
  removed: string[];
  still_wanted: boolean;
}

export type Mode = 'full' | 'latest' | `last:${number}`;

// --- extension <-> webview protocol -----------------------------------------

export type Action =
  | { kind: 'connect'; ticket: string }
  | { kind: 'pull'; reference: string }
  | { kind: 'announce'; reference?: string }
  | { kind: 'sync'; peer?: string }
  | { kind: 'gc'; dryRun: boolean; force: boolean }
  | { kind: 'rm'; reference: string; allTags: boolean }
  | { kind: 'seed'; reference: string; mode: Mode }
  | { kind: 'unseed'; reference: string }
  | { kind: 'follow'; publisher: string; mode: Mode }
  | { kind: 'unfollow'; publisher: string }
  | { kind: 'pin'; reference: string }
  | { kind: 'unpin'; reference: string };

export interface WebviewIn {
  type: 'ready';
}

export interface WebviewAction {
  type: 'action';
  action: Action;
}

export type WebviewMessage = WebviewIn | WebviewAction;

export interface StateSnapshot {
  daemon: boolean;
  status?: Status;
  releases?: ReleaseInfo[];
  peers?: PeerInfo[];
  /** Ring buffer of the most recent daemon events, oldest first. */
  events: DaemonEvent[];
  /** Human-readable failure of the last action, if any. */
  error?: string;
}

export interface WebviewOut {
  type: 'state';
  state: StateSnapshot;
}
