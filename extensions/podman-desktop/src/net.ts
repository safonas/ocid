// Minimal HTTP(S) layer over node:http/node:https — global `fetch` cannot be
// given a per-process CA, which trusting a self-signed daemon CA requires.

import { request as httpRequest } from 'node:http';
import { Agent, request as httpsRequest } from 'node:https';
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

/** Agents carrying a trusted daemon CA, keyed by the PEM. Podman Desktop's
 *  extension host swaps node's global https agent for its own, which
 *  silently ignores per-request `ca` options — so the CA must travel inside
 *  a custom agent (verified empirically: agent-carried CA connects, the
 *  identical per-request `ca` fails with `unable to verify the first
 *  certificate`). */
const caAgents = new Map<string, Agent>();

function httpsAgent(ca: string): Agent {
  let agent = caAgents.get(ca);
  if (!agent) {
    agent = new Agent({ ca });
    caAgents.set(ca, agent);
  }
  return agent;
}

function send(url: URL, opts: NetOptions): Promise<Response> {
  const base = { method: opts.method ?? 'GET', headers: opts.headers };
  const httpsOpts =
    opts.ca !== undefined ? { ...base, agent: httpsAgent(opts.ca) } : { ...base };
  return new Promise((resolve, reject) => {
    const req =
      url.protocol === 'https:'
        ? httpsRequest(url, httpsOpts, res => resolve(toResponse(res)))
        : httpRequest(url, { ...base }, res => resolve(toResponse(res)));
    req.on('error', reject);
    if (opts.body) req.write(opts.body);
    req.end();
  });
}

function toResponse(res: IncomingMessage): Response {
  return { status: res.statusCode ?? 0, stream: res };
}

/** Full-body request (small JSON responses). */
export async function requestText(
  url: URL,
  opts: NetOptions = {},
): Promise<{ status: number; body: string }> {
  const { status, stream } = await send(url, opts);
  const chunks: Buffer[] = [];
  for await (const chunk of stream) chunks.push(chunk as Buffer);
  return { status, body: Buffer.concat(chunks).toString('utf8') };
}

/** Streamed GET (the SSE feed): `onChunk` receives raw bytes; stops early
 *  when `shouldStop()` is true, resolving with the response status. */
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
