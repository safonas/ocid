// The daemon's self-signed CA: found on disk and installed into podman's
// certs.d so push/pull verify the https registry without --tls-verify=false.
// The TLS counterpart of the registries.ts insecure drop-in for plain http.

import { mkdir, readFile, rename, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import path from 'node:path';

import { configHome } from './registries.ts';

const CA_FILE = 'ca.crt';

/** CA locations in preference order: extension pod home, the deb/rpm system
 *  service (/var/lib/ocid — ca.crt is world-readable there), then ~/.ocid. */
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

/** Re-read the CA from disk — the setup poll calls this, so a just-started
 *  daemon is trusted without an extension restart. */
export async function refreshCa(): Promise<void> {
  cachedCa = await findCa();
}

export function getCa(): string | undefined {
  return cachedCa;
}

export function caDir(host: string, home: string = configHome()): string {
  return path.join(home, 'containers', 'certs.d', host);
}

export function caPath(host: string, home: string = configHome()): string {
  return path.join(caDir(host, home), CA_FILE);
}

/** Install the CA into certs.d (idempotent). */
export async function installCa(
  host: string,
  pem: string,
  home: string = configHome(),
): Promise<boolean> {
  const file = caPath(host, home);
  try {
    if ((await readFile(file, 'utf8')) === pem) return true;
  } catch {
    // not present — write it below
  }
  await mkdir(path.dirname(file), { recursive: true });
  const tmp = path.join(path.dirname(file), `${CA_FILE}.tmp-${process.pid}`);
  await writeFile(tmp, pem);
  await rename(tmp, file);
  return true;
}
