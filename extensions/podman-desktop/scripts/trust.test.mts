// Unit tests for the certs.d trust module (no daemon needed):
// run via `just ext-test` (node --experimental-strip-types --test).

import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { caDir, caPath, installCa } from '../src/trust.ts';

const PEM = '-----BEGIN CERTIFICATE-----\nZm9v\n-----END CERTIFICATE-----\n';

test('caPath lives in certs.d/<host>', () => {
  assert.equal(
    caPath('127.0.0.1:5050', '/home/user/.config'),
    path.join('/home/user/.config', 'containers', 'certs.d', '127.0.0.1:5050', 'ca.crt'),
  );
  assert.equal(
    caDir('localhost:5050', '/home/user/.config'),
    path.join('/home/user/.config', 'containers', 'certs.d', 'localhost:5050'),
  );
});

test('installCa writes the CA and is idempotent', async () => {
  const home = await mkdtemp(path.join(tmpdir(), 'ocid-trust-'));
  const { readdir } = await import('node:fs/promises');
  try {
    assert.equal(await installCa('127.0.0.1:5050', PEM, home), true);
    assert.equal(await readFile(caPath('127.0.0.1:5050', home), 'utf8'), PEM);
    // second install is a no-op and leaves no temp files behind
    assert.equal(await installCa('127.0.0.1:5050', PEM, home), true);
    assert.deepEqual(await readdir(caDir('127.0.0.1:5050', home)), ['ca.crt']);
    // a changed CA is rewritten
    const next = PEM.replace('Zm9v', 'YmFy');
    assert.equal(await installCa('127.0.0.1:5050', next, home), true);
    assert.equal(await readFile(caPath('127.0.0.1:5050', home), 'utf8'), next);
  } finally {
    await rm(home, { recursive: true, force: true });
  }
});
