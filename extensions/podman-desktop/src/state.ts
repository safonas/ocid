// Owns every daemon interaction for the dashboard: polls state (like ocitop,
// 2s), appends live SSE events, dispatches webview actions, and pushes
// snapshots to the webview. The webview is a pure view of this state.

import type {
  Action,
  DaemonEvent,
  StateSnapshot,
  Status,
  WebviewMessage,
} from './types';
import type { PeerInfo, ReleaseInfo } from './types';
import { OcidClient, OcidError } from './ocid-client';
import { EventRing, SseWatcher } from './sse';
import { TransferTracker } from './transfers';

const POLL_MS = 2_000;

interface Dependencies {
  client: OcidClient;
  webview: WebviewLike;
  confirm: (message: string, ok: string) => Promise<boolean>;
  notify: (message: string, error: boolean) => void;
}

/** The slice of the podman-desktop webview API this state talks to. */
export interface WebviewLike {
  postMessage(message: unknown): Promise<boolean> | boolean;
}

export class DashboardState {
  private readonly ring = new EventRing();
  private readonly sse: SseWatcher;
  private readonly deps: Dependencies;
  private readonly transfers = new TransferTracker();
  private snapshot: StateSnapshot = { daemon: false, events: [], transfers: [] };
  private timer: NodeJS.Timeout | undefined;
  private polling = false;

  constructor(deps: Dependencies) {
    this.deps = deps;
    this.sse = new SseWatcher(
      deps.client.base,
      this.ring,
      () => this.push(),
      ev => this.onEvent(ev),
    );
  }

  start(): void {
    this.sse.start();
    this.timer = setInterval(() => void this.poll(), POLL_MS);
    void this.poll();
  }

  dispose(): void {
    this.sse.stop();
    if (this.timer) clearInterval(this.timer);
    this.timer = undefined;
  }

  private async poll(): Promise<void> {
    if (this.polling) return;
    this.polling = true;
    try {
      const [status, releases, peers] = await Promise.all([
        this.deps.client.status(),
        this.deps.client.releases(),
        this.deps.client.peers(),
      ]);
      this.transfers.prune();
      this.snapshot = {
        daemon: true,
        status,
        releases,
        peers,
        events: this.ring.snapshot(),
        transfers: this.transfers.list(),
        error: this.snapshot.error,
      };
      this.push();
    } catch {
      const wasUp = this.snapshot.daemon;
      this.transfers.clear();
      this.snapshot = {
        daemon: false,
        events: this.ring.snapshot(),
        transfers: [],
        error: wasUp ? undefined : this.snapshot.error,
      };
      this.ring.clear();
      this.push();
    } finally {
      this.polling = false;
    }
  }

  /** SSE events drive the live views (transfers strip, event stream)
   *  immediately, without waiting for the next poll. */
  private onEvent(ev: DaemonEvent): void {
    this.transfers.onEvent(ev);
    this.snapshot = {
      ...this.snapshot,
      events: this.ring.snapshot(),
      transfers: this.transfers.list(),
    };
  }

  push(): void {
    void this.deps.webview.postMessage({ type: 'state', state: this.snapshot });
  }

  async handleMessage(message: WebviewMessage): Promise<void> {
    switch (message.type) {
      case 'ready':
        await this.poll();
        break;
      case 'action':
        await this.run(message.action);
        break;
    }
  }

  private async run(action: Action): Promise<void> {
    const c = this.deps.client;
    let detail = '';
    try {
      switch (action.kind) {
        case 'connect': {
          const r = await c.connect(action.ticket);
          detail = `connected to ${short(r.id)}`;
          break;
        }
        case 'pull':
          await c.pull(action.reference);
          detail = `pulled ${action.reference}`;
          break;
        case 'announce': {
          const r = await c.announce(action.reference);
          detail = `announced ${r.announced} release(s)`;
          break;
        }
        case 'sync': {
          const r = await c.sync(action.peer);
          detail = r.failed.length
            ? `synced with ${r.synced} peer(s); failed: ${r.failed.join(', ')}`
            : `synced with ${r.synced} peer(s)`;
          break;
        }
        case 'gc': {
          if (action.force && !(await this.deps.confirm('Run garbage collection ignoring the grace period?', 'Collect'))) {
            return;
          }
          const r = await c.gc(action.dryRun, action.force);
          detail = r.dry_run
            ? `GC dry-run: would remove ${r.releases_removed.length} release(s), ${r.blobs_removed} blob(s), ${fmtBytes(r.bytes_freed)}`
            : `GC removed ${r.releases_removed.length} release(s), ${r.blobs_removed} blob(s), ${fmtBytes(r.bytes_freed)}`;
          break;
        }
        case 'rm': {
          const what = action.allTags ? `every tag of ${action.reference}` : action.reference;
          if (!(await this.deps.confirm(`Remove ${what}? (local only; not propagated)`, 'Remove'))) {
            return;
          }
          const r = await c.rm(action.reference, action.allTags);
          detail = r.removed.length ? `removed ${r.removed.join(', ')}` : 'nothing removed';
          if (r.still_wanted) {
            detail += ' — the policy still wants it; it will be replicated again';
          }
          break;
        }
        case 'seed': {
          const r = await c.seed(action.reference, action.mode);
          detail = `${r.changed ? 'seeding' : 'already seeding'} ${r.reference}`;
          break;
        }
        case 'unseed': {
          const r = await c.unseed(action.reference);
          detail = `${r.changed ? 'no longer seeding' : 'was not seeding'} ${r.reference}`;
          break;
        }
        case 'follow': {
          const r = await c.follow(action.publisher, action.mode);
          detail = `${r.changed ? 'following' : 'already following'} ${short(r.reference)} (${action.mode})`;
          break;
        }
        case 'unfollow': {
          const r = await c.unfollow(action.publisher);
          detail = `${r.changed ? 'unfollowed' : 'was not following'} ${short(r.reference)}`;
          break;
        }
        case 'pin': {
          const r = await c.pin(action.reference);
          detail = `${r.changed ? 'pinned' : 'already pinned'} ${r.reference}`;
          break;
        }
        case 'unpin': {
          const r = await c.unpin(action.reference);
          detail = `${r.changed ? 'unpinned' : 'was not pinned'} ${r.reference}`;
          break;
        }
      }
      this.snapshot = { ...this.snapshot, error: undefined };
      this.deps.notify(detail, false);
    } catch (e) {
      const msg = e instanceof OcidError ? e.message : (e as Error).message;
      this.snapshot = { ...this.snapshot, error: msg };
      this.deps.notify(msg, true);
    }
    await this.poll();
  }

  current(): StateSnapshot {
    return this.snapshot;
  }
}

export function short(id: string): string {
  return `${id.slice(0, 12)}…`;
}

export function fmtBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KiB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MiB`;
  return `${(n / 1024 ** 3).toFixed(1)} GiB`;
}

export type { DaemonEvent, PeerInfo, ReleaseInfo, Status };
