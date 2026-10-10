// Unit tests for the dashboard pull-field input normalization (no
// daemon): run via `just ext-test` (node --experimental-strip-types
// --test).

import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizePullRef } from '../src/pullref.ts';

test('bare references pass through untouched', () => {
  assert.equal(normalizePullRef('ocid.dev/app:1.0'), 'ocid.dev/app:1.0');
  assert.equal(
    normalizePullRef('  ef34d1b486e955f6/app:v0.8.0-alpha.1  '),
    'ef34d1b486e955f6/app:v0.8.0-alpha.1',
  );
});

test('pasted pull commands lose the command, keep the reference', () => {
  assert.equal(
    normalizePullRef('podman pull 127.0.0.1:5050/ocid.dev/app:1.0'),
    '127.0.0.1:5050/ocid.dev/app:1.0',
  );
  assert.equal(normalizePullRef('docker pull ocid.dev/app:1.0'), 'ocid.dev/app:1.0');
});

test('whitespace is trimmed, non-commands are left alone', () => {
  assert.equal(normalizePullRef('   '), '');
  assert.equal(normalizePullRef('podman'), 'podman');
});
