// Unit tests for the Images-page menu argument handling (no daemon):
// run via `just ext-test` (node --experimental-strip-types --test).
//
// PD passes its ImageInfoUI object to dashboard/image menu commands
// (name/tag/engineName), not the api.ImageInfo (RepoTags/engineType) the
// API types suggest — both shapes must work.

import test from 'node:test';
import assert from 'node:assert/strict';
import { isPodmanEngine, menuImageSource, ocidName } from '../src/images.ts';

test('menuImageSource reads the UI shape (name + tag)', () => {
  assert.equal(
    menuImageSource({ name: 'docker.io/library/alpine', tag: 'latest' }),
    'docker.io/library/alpine:latest',
  );
  assert.equal(menuImageSource({ name: 'localhost/myapp', tag: 'dev' }), 'localhost/myapp:dev');
});

test('menuImageSource reads the api.ImageInfo shape (RepoTags)', () => {
  assert.equal(
    menuImageSource({ RepoTags: ['quay.io/org/app:1.0'] }),
    'quay.io/org/app:1.0',
  );
});

test('menuImageSource rejects untagged and missing images', () => {
  assert.equal(menuImageSource(undefined), undefined);
  assert.equal(menuImageSource({ name: '<none>', tag: '' }), undefined);
  assert.equal(menuImageSource({ RepoTags: ['<none>:<none>'] }), undefined);
  assert.equal(menuImageSource({}), undefined);
});

test('isPodmanEngine accepts both engine fields', () => {
  assert.equal(isPodmanEngine({ engineType: 'podman' }), true);
  assert.equal(isPodmanEngine({ engineName: 'podman' }), true);
  assert.equal(isPodmanEngine({ engineType: 'docker' }), false);
  assert.equal(isPodmanEngine({ engineName: 'docker' }), false);
  assert.equal(isPodmanEngine(undefined), false);
});

test('ocidName strips registry and library prefixes', () => {
  assert.equal(ocidName('docker.io/library/alpine:latest'), 'alpine:latest');
  assert.equal(ocidName('quay.io/org/app'), 'org/app');
  assert.equal(ocidName('localhost/foo:dev'), 'foo:dev');
  assert.equal(ocidName('127.0.0.1:5050/foo:dev'), 'foo:dev');
  assert.equal(ocidName('myapp:latest'), 'myapp:latest');
  assert.equal(ocidName('ghcr.io/user/app:v2'), 'user/app:v2');
});
