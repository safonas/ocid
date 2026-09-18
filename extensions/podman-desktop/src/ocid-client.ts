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

  constructor(base: string) {
    this.base = base;
  }

  private async request<T>(path: string, init?: RequestInit): Promise<T> {
    let resp: Response;
    try {
      resp = await fetch(`${this.base}${path}`, init);
    } catch (e) {
      throw new OcidError(`cannot reach the ocid daemon at ${this.base}: ${(e as Error).message}`, undefined);
    }
    const text = await resp.text();
    if (!resp.ok) {
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
      throw new OcidError(msg ?? `HTTP ${resp.status}`, resp.status);
    }
    return JSON.parse(text) as T;
  }

  private post<T>(path: string, body: unknown): Promise<T> {
    return this.request<T>(path, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
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
