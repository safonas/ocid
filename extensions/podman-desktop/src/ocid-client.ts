// Typed client for the daemon's /_ocid/ control API (crates/ocid-core/src/api.rs).
// All network I/O lives in the extension backend; the webview never fetches.

import type {
  GcReport,
  Mode,
  PeerInfo,
  PolicyChangeResp,
  ReleaseInfo,
  RmResp,
  Status,
  SyncResp,
} from './types';
import { requestText } from './net.ts';

export class OcidError extends Error {
  readonly status: number | undefined;

  constructor(
    message: string,
    status: number | undefined,
  ) {
    super(message);
    this.status = status;
  }
}

export class OcidClient {
  readonly base: string;
  readonly ca: string | undefined;

  constructor(base: string, ca?: string) {
    this.base = base;
    this.ca = ca;
  }

  private async request<T>(
    path: string,
    init?: { method?: 'GET' | 'POST'; body?: string },
  ): Promise<T> {
    let status: number;
    let text: string;
    try {
      ({ status, body: text } = await requestText(new URL(`${this.base}${path}`), {
        method: init?.method,
        headers: init?.method === 'POST' ? { 'content-type': 'application/json' } : undefined,
        body: init?.body,
        ca: this.ca,
      }));
    } catch (e) {
      throw new OcidError(
        `cannot reach the ocid daemon at ${this.base}: ${(e as Error).message}`,
        undefined,
      );
    }
    if (status < 200 || status >= 300) {
      const msg =
        text &&
        (() => {
          try {
            const v = JSON.parse(text) as { error?: string };
            return v.error;
          } catch {
            return undefined;
          }
        })();
      throw new OcidError(msg ?? `HTTP ${status}`, status);
    }
    return JSON.parse(text) as T;
  }

  private post<T>(path: string, body: unknown): Promise<T> {
    return this.request<T>(path, {
      method: 'POST',
      body: JSON.stringify(body),
    });
  }

  status(): Promise<Status> {
    return this.request<Status>('/_ocid/status');
  }

  releases(): Promise<ReleaseInfo[]> {
    return this.request<ReleaseInfo[]>('/_ocid/releases');
  }

  peers(): Promise<PeerInfo[]> {
    return this.request<PeerInfo[]>('/_ocid/peers');
  }

  connect(ticket: string): Promise<{ id: string }> {
    return this.post('/_ocid/peers', { ticket });
  }

  pull(reference: string): Promise<unknown> {
    return this.post('/_ocid/pull', { reference });
  }

  announce(reference?: string): Promise<{ announced: number }> {
    return this.post('/_ocid/announce', { reference });
  }

  sync(peer?: string): Promise<SyncResp> {
    return this.post('/_ocid/sync', { peer: peer ?? null });
  }

  gc(dryRun: boolean, force: boolean): Promise<GcReport> {
    return this.post('/_ocid/gc', { dry_run: dryRun, force });
  }

  rm(reference: string, allTags: boolean): Promise<RmResp> {
    return this.post('/_ocid/rm', { reference, all_tags: allTags });
  }

  seed(reference: string, mode: Mode): Promise<PolicyChangeResp> {
    return this.post('/_ocid/policy/seed', { reference, mode });
  }

  unseed(reference: string): Promise<PolicyChangeResp> {
    return this.post('/_ocid/policy/unseed', { reference });
  }

  follow(publisher: string, mode: Mode): Promise<PolicyChangeResp> {
    return this.post('/_ocid/policy/follow', { publisher, mode });
  }

  unfollow(publisher: string): Promise<PolicyChangeResp> {
    return this.post('/_ocid/policy/unfollow', { publisher });
  }

  pin(reference: string): Promise<PolicyChangeResp> {
    return this.post('/_ocid/policy/pin', { reference });
  }

  unpin(reference: string): Promise<PolicyChangeResp> {
    return this.post('/_ocid/policy/unpin', { reference });
  }
}
