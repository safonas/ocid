// Locates the daemon's self-signed CA and installs it into podman's certs.d
// so `podman push/pull` verifies the https registry without --tls-verify=false.
//
// This is the TLS counterpart of the registries.conf drop-in in registries.ts:
// that one marks a *plain-http* registry insecure, this one trusts the CA of a
// *https* registry. certs.d/<host:port>/ca.crt is the per-registry location
// podman (containers/image) reads by default.

import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';

import { configHome } from './registries.ts';

const CA_FILE = 'ca.crt';

/** Candidate OCID_HOME TLS dirs, in preference order: the extension-managed
 *  pod home, the deb/rpm system service's StateDirectory (/var/lib/ocid —
 *  its tls/ca.crt is world-readable even though the key is not), then the
 *  default `~/.ocid`. */
export function caCandidates(): string[] {
  const home = homedir();
  return [
    path.join(home, '.local', 'share', 'ocid', 'tls', CA_FILE),
    '/var/lib/ocid/tls/ca.crt',
    path.join(home, '.ocid', 'tls', CA_FILE),
  ];
}

/** PEM of the daemon's CA, if a TLS daemon's home can be found on disk. */
export async function findCa(): Promise<string | undefined> {
  for (const candidate of caCandidates()) {
    try {
      const pem = await readFile(candidate, 'utf8');
      if (pem.includes('BEGIN CERTIFICATE')) return pem;
    } catch {
      // keep looking
    }
  }
  return undefined;
}

let cachedCa: string | undefined;

/** Re-read the CA from disk. Called on activation and on the 2s setup poll,
 *  so the client trusts a just-started daemon without a restart (the CA does
 *  not exist until the daemon first generates it). */
export async function refreshCa(): Promise<void> {
  cachedCa = await findCa();
}

/** The last-known CA, or undefined before the daemon has generated one. */
export function getCa(): string | undefined {
  return cachedCa;
}

/** certs.d directory for one registry host:port. */
export function caDir(host: string, home: string = configHome()): string {
  return path.join(home, 'containers', 'certs.d', host);
}

export function caPath(host: string, home: string = configHome()): string {
  return path.join(caDir(host, home), CA_FILE);
}

/** Install the CA into certs.d (idempotent); true when it is in place. */
export async function installCa(
  host: string,
  pem: string,
  home: string = configHome(),
): Promise<boolean> {
  const file = caPath(host, home);
  try {
    if ((await readFile(file, 'utf8')) === pem) return true;
  } catch {
    // not present (or unreadable) — (re)write it below
  }
  await mkdir(path.dirname(file), { recursive: true });
  const tmp = path.join(path.dirname(file), `${CA_FILE}.tmp-${process.pid}`);
  await writeFile(tmp, pem);
  await rename(tmp, file);
  return true;
}
