// Minimal HTTP(S) layer over node:http/node:https. The global `fetch` cannot
// be given a per-process CA at runtime, so the extension talks to the daemon
// through this instead — which is also what lets a TLS daemon's self-signed
// CA be trusted.

import { request as httpRequest } from 'node:http';
import { request as httpsRequest } from 'node:https';
import type { IncomingMessage } from 'node:http';

export interface NetOptions {
  method?: 'GET' | 'POST';
  headers?: Record<string, string>;
  body?: string;
  /** PEM CA to trust for https (undefined = system roots only). */
  ca?: string;
}

interface Response {
  status: number;
  stream: IncomingMessage;
}

function send(url: URL, opts: NetOptions): Promise<Response> {
  const base = { method: opts.method ?? 'GET', headers: opts.headers };
  return new Promise((resolve, reject) => {
    const req =
      url.protocol === 'https:'
        ? httpsRequest(url, { ...base, ca: opts.ca }, res => resolve(toResponse(res)))
        : httpRequest(url, { ...base }, res => resolve(toResponse(res)));
    req.on('error', reject);
    if (opts.body) req.write(opts.body);
    req.end();
  });
}

function toResponse(res: IncomingMessage): Response {
  return { status: res.statusCode ?? 0, stream: res };
}

/** Full-body request (small JSON control API responses). */
export async function requestText(
  url: URL,
  opts: NetOptions = {},
): Promise<{ status: number; body: string }> {
  const { status, stream } = await send(url, opts);
  const chunks: Buffer[] = [];
  for await (const chunk of stream) chunks.push(chunk as Buffer);
  return { status, body: Buffer.concat(chunks).toString('utf8') };
}

/** Streamed GET (the SSE event feed). `onChunk` receives raw bytes; resolve
 *  with the response status when the stream ends or `shouldStop()` is true. */
export async function requestStream(
  url: URL,
  opts: NetOptions,
  onChunk: (chunk: Buffer) => void,
  shouldStop?: () => boolean,
): Promise<number> {
  const { status, stream } = await send(url, opts);
  for await (const chunk of stream) {
    onChunk(chunk as Buffer);
    if (shouldStop?.()) {
      stream.destroy();
      break;
    }
  }
  return status;
}
