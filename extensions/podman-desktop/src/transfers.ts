// Live-transfer view model: derives TransferState rows for the webview from
// daemon events (pull_progress / release_saved / fetch_failed). Pure
// bookkeeping — no I/O; `now` is injectable for tests.

import type { DaemonEvent, TransferState } from './types';

/** Finished transfers linger this long before dropping off the strip. */
const TRANSFER_LINGER_MS = 5_000;
/** Actives with no progress for this long are abandoned (daemon died mid-pull). */
const TRANSFER_STALE_MS = 60_000;
/** Below this gap between progress events, the byte rate is not recomputed. */
const RATE_MIN_DT_MS = 250;
/** EWMA smoothing factor for the transfer byte rate. */
const RATE_ALPHA = 0.3;

export class TransferTracker {
  private readonly map = new Map<string, TransferState>();

  onEvent(ev: DaemonEvent, now: number = Date.now()): void {
    switch (ev.type) {
      case 'pull_progress': {
        const key = `${ev.publisher}/${ev.name}:${ev.tag}`;
        const prev = this.map.get(key);
        let bytesPerSec = prev?.bytesPerSec ?? 0;
        if (prev && now > prev.updatedAt) {
          const dt = now - prev.updatedAt;
          const dBytes = ev.bytes_done - prev.bytesDone;
          if (dt >= RATE_MIN_DT_MS && dBytes >= 0) {
            const instant = (dBytes * 1000) / dt;
            bytesPerSec =
              prev.bytesPerSec > 0
                ? RATE_ALPHA * instant + (1 - RATE_ALPHA) * prev.bytesPerSec
                : instant;
          }
        }
        this.map.set(key, {
          key,
          publisher: ev.publisher,
          name: ev.name,
          tag: ev.tag,
          state: 'active',
          blobsDone: ev.blobs_done,
          blobsTotal: ev.blobs_total,
          bytesDone: ev.bytes_done,
          bytesTotal: ev.bytes_total,
          bytesPerSec,
          updatedAt: now,
        });
        break;
      }
      case 'release_saved': {
        const key = `${ev.publisher}/${ev.name}:${ev.tag}`;
        const t = this.map.get(key);
        if (t?.state === 'active') {
          this.map.set(key, {
            ...t,
            state: 'done',
            blobsDone: t.blobsTotal,
            bytesDone: t.bytesTotal,
            bytesPerSec: 0,
            finishedAt: now,
            updatedAt: now,
          });
        }
        break;
      }
      case 'fetch_failed': {
        const key = `${ev.publisher}/${ev.name}:${ev.tag}`;
        const t = this.map.get(key);
        if (t?.state === 'active') {
          this.map.set(key, {
            ...t,
            state: 'failed',
            error: ev.error,
            bytesPerSec: 0,
            finishedAt: now,
            updatedAt: now,
          });
        }
        break;
      }
      default:
        break;
    }
    this.prune(now);
  }

  prune(now: number = Date.now()): void {
    for (const [key, t] of this.map) {
      if (t.finishedAt !== undefined && now - t.finishedAt > TRANSFER_LINGER_MS) {
        this.map.delete(key);
      } else if (t.state === 'active' && now - t.updatedAt > TRANSFER_STALE_MS) {
        this.map.delete(key);
      }
    }
  }

  /** Newest update first. */
  list(): TransferState[] {
    return [...this.map.values()].sort((a, b) => b.updatedAt - a.updatedAt);
  }

  clear(): void {
    this.map.clear();
  }
}
