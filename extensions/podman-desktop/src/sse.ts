// Consumes the daemon's GET /_ocid/events SSE stream (DaemonEvent as JSON
// `data:` lines), keeping a bounded ring buffer. Polling — not SSE — is the
// primary data path, so failures here only mean "no live view".

import type { DaemonEvent, TimedEvent } from './types';
import { requestStream } from './net.ts';

const MAX_EVENTS = 200;
const RECONNECT_DELAY_MS = 5_000;

export class EventRing {
  private buf: TimedEvent[] = [];

  push(...events: DaemonEvent[]): void {
    const receivedAt = Date.now();
    for (const ev of events) {
      // The daemon does not timestamp events; stamp on receipt so the
      // webview can render a time gutter.
      this.buf.push({ ...ev, receivedAt } as TimedEvent);
    }
    if (this.buf.length > MAX_EVENTS) {
      this.buf = this.buf.slice(-MAX_EVENTS);
    }
  }

  snapshot(): TimedEvent[] {
    return [...this.buf];
  }

  clear(): void {
    this.buf = [];
  }
}

export class SseWatcher {
  #stopped = false;
  private readonly base: string;
  private readonly ca: string | undefined;
  private readonly ring: EventRing;
  private readonly onChange: () => void;
  private readonly onEvent: (event: DaemonEvent) => void;

  constructor(
    base: string,
    ring: EventRing,
    onChange: () => void,
    onEvent: (event: DaemonEvent) => void,
    ca?: string,
  ) {
    this.base = base;
    this.ca = ca;
    this.ring = ring;
    this.onChange = onChange;
    this.onEvent = onEvent;
  }

  start(): void {
    void this.loop();
  }

  stop(): void {
    this.#stopped = true;
  }

  private async loop(): Promise<void> {
    while (!this.#stopped) {
      try {
        await this.streamOnce();
      } catch {
        // daemon down or stream ended; retry after a pause
      }
      if (this.#stopped) return;
      await new Promise(resolve => setTimeout(resolve, RECONNECT_DELAY_MS));
    }
  }

  private async streamOnce(): Promise<void> {
    const url = new URL(`${this.base}/_ocid/events`);
    const decoder = new TextDecoder();
    let buffer = '';
    const status = await requestStream(
      url,
      { headers: { accept: 'text/event-stream' }, ca: this.ca },
      chunk => {
        // Buffer raw text: a chunk may split an SSE frame or a UTF-8 char.
        buffer += decoder.decode(chunk, { stream: true });
        let sep: number;
        while ((sep = buffer.indexOf('\n\n')) !== -1) {
          const frame = buffer.slice(0, sep);
          buffer = buffer.slice(sep + 2);
          this.handleFrame(frame);
        }
      },
      () => this.#stopped,
    );
    if (status !== 200) {
      throw new Error(`event stream returned HTTP ${status}`);
    }
  }

  private handleFrame(frame: string): void {
    const data = frame
      .split('\n')
      .filter(line => line.startsWith('data:'))
      .map(line => line.slice(5).trimStart())
      .join('\n');
    if (!data || data === '[DONE]') return;
    let event: DaemonEvent;
    try {
      event = JSON.parse(data) as DaemonEvent;
    } catch {
      return;
    }
    if (typeof event !== 'object' || event === null || !('type' in event)) return;
    this.ring.push(event);
    this.onEvent(event);
    this.onChange();
  }
}
