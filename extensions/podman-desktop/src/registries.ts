// Registers the local ocid registry with podman so `podman push/pull`
// against 127.0.0.1:5050 works without --tls-verify=false.
//
// The ocid registry is plain http on loopback, which podman only accepts
// through a `registries.conf` entry with `insecure = true`. Podman Desktop's
// Settings → Registries dialog cannot express this (it drives an auth.json
// login flow for https registries and demands credentials), so the extension
// owns a drop-in file in the user's registries.conf.d instead — the mechanism
// the Podman Desktop docs prescribe for manual setup.
//
// The drop-in is user-level: rootless podman reads it, but rootful podman
// (and podman machines on other platforms) does not — pushes/pulls from
// those fall back to --tls-verify=false in extension.ts.

import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';

const DROPIN_NAME = '100-ocid.conf';

/** Registry host:port in the form podman sees (no scheme, no trailing slash). */
export function registryHost(url: string): string {
  try {
    const parsed = new URL(url);
    // Only http(s) URLs carry the host in `.host`; anything else that
    // happens to parse (e.g. "localhost:5050" → scheme "localhost") does not.
    if (parsed.protocol === 'http:' || parsed.protocol === 'https:') return parsed.host;
  } catch {
    // not a URL — strip a possible scheme prefix below
  }
  return url.replace(/^https?:\/\//, '').replace(/\/+$/, '');
}

/** User-level containers config dir (what rootless podman reads). */
export function configHome(): string {
  return process.env['XDG_CONFIG_HOME'] || path.join(homedir(), '.config');
}

export function dropinPath(home: string = configHome()): string {
  return path.join(home, 'containers', 'registries.conf.d', DROPIN_NAME);
}

export function dropinContent(host: string): string {
  return [
    '# Managed by the ocid Podman Desktop extension.',
    '[[registry]]',
    `location = "${host}"`,
    'insecure = true',
    '',
  ].join('\n');
}

/** True when the drop-in registers this exact host (a stale host from a
 *  changed ocid.registryUrl setting does not count). */
export async function isRegistered(host: string, home: string = configHome()): Promise<boolean> {
  try {
    const content = await readFile(dropinPath(home), 'utf8');
    return content === dropinContent(host);
  } catch {
    return false;
  }
}

/** Write the drop-in atomically (temp file + rename), creating the
 *  registries.conf.d directory if needed. Rewriting an existing drop-in
 *  with a new host is how a changed ocid.registryUrl setting propagates. */
export async function register(host: string, home: string = configHome()): Promise<void> {
  const file = dropinPath(home);
  await mkdir(path.dirname(file), { recursive: true });
  const tmp = path.join(path.dirname(file), `${DROPIN_NAME}.tmp-${process.pid}`);
  await writeFile(tmp, dropinContent(host));
  await rename(tmp, file);
}
