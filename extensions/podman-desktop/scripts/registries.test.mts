// Unit tests for the registries.conf drop-in module (no daemon needed):
// run via `just ext-test` (node --experimental-strip-types --test).

import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import {
  dropinContent,
  dropinPath,
  isRegistered,
  register,
  registryHost,
} from '../src/registries.ts';

test('registryHost strips scheme, path and trailing slash', () => {
  assert.equal(registryHost('http://127.0.0.1:5050'), '127.0.0.1:5050');
  assert.equal(registryHost('http://127.0.0.1:5050/'), '127.0.0.1:5050');
  assert.equal(registryHost('https://reg.example.com:8443/v1/'), 'reg.example.com:8443');
  assert.equal(registryHost('127.0.0.1:5050'), '127.0.0.1:5050');
  assert.equal(registryHost('localhost:5050/'), 'localhost:5050');
});

test('dropinPath lives in registries.conf.d with a stable name', () => {
  assert.equal(
    dropinPath('/home/user/.config'),
    path.join('/home/user/.config', 'containers', 'registries.conf.d', '100-ocid.conf'),
  );
});

test('dropinContent marks the host insecure', () => {
  assert.equal(
    dropinContent('127.0.0.1:5050'),
    [
      '# Managed by the ocid Podman Desktop extension.',
      '[[registry]]',
      'location = "127.0.0.1:5050"',
      'insecure = true',
      '',
    ].join('\n'),
  );
});

test('register writes the drop-in and isRegistered sees it', async () => {
  const home = await mkdtemp(path.join(tmpdir(), 'ocid-registries-'));
  try {
    assert.equal(await isRegistered('127.0.0.1:5050', home), false);
    await register('127.0.0.1:5050', home);
    assert.equal(await isRegistered('127.0.0.1:5050', home), true);
    assert.equal(await readFile(dropinPath(home), 'utf8'), dropinContent('127.0.0.1:5050'));

    // A stale drop-in for a different host does not count as registered.
    assert.equal(await isRegistered('127.0.0.1:5051', home), false);
    // Re-registering with a new host replaces the drop-in (setting changes).
    await register('127.0.0.1:5051', home);
    assert.equal(await isRegistered('127.0.0.1:5051', home), true);
    assert.equal(await isRegistered('127.0.0.1:5050', home), false);
  } finally {
    await rm(home, { recursive: true, force: true });
  }
});

test('register is idempotent, creates dirs, and leaves no temp files', async () => {
  const home = await mkdtemp(path.join(tmpdir(), 'ocid-registries-'));
  const dir = path.join(home, 'containers', 'registries.conf.d');
  const { readdir } = await import('node:fs/promises');
  try {
    await register('localhost:5050', home);
    await register('localhost:5050', home);
    assert.deepEqual(await readdir(dir), ['100-ocid.conf']);
  } finally {
    await rm(home, { recursive: true, force: true });
  }
});
