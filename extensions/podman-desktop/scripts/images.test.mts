// Unit tests for the Images-page menu argument handling (no daemon):
// run via `just ext-test` (node --experimental-strip-types --test).
//
// PD passes its ImageInfoUI object to dashboard/image menu commands
// (name/tag/engineName), not the api.ImageInfo (RepoTags/engineType) the
// API types suggest — both shapes must work.

import test from 'node:test';
import assert from 'node:assert/strict';
import { isPodmanEngine, menuImageSource, ocidName, ocidTarget } from '../src/images.ts';

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

test('ocidTarget keeps tagged references', () => {
  assert.equal(ocidTarget('docker.io/library/alpine:latest'), 'alpine:latest');
  assert.equal(ocidTarget('quay.io/org/app:1.0'), 'org/app:1.0');
  assert.equal(ocidTarget('myapp'), 'myapp');
});

test('ocidTarget derives a tag for digest-pinned references', () => {
  // The shape the Images page sends for digest-only images: the UI splits
  // name/tag on the last ':', so name carries '@sha256' and tag is the hex.
  assert.equal(
    ocidTarget('cgr.dev/chainguard/node@sha256:2a2df3a1f79cfe63317e3e6e2b7394cac434648fdfc82e4f702d0e6dc4d9a852'),
    'chainguard/node:sha256-2a2df3a1f79c',
  );
  assert.equal(ocidTarget('localhost/foo@sha256:abcd'), 'foo:sha256-abcd');
});

test('ocidTarget keeps the real tag of name:tag@digest references', () => {
  assert.equal(ocidTarget('quay.io/org/app:1.0@sha256:abcd'), 'org/app:1.0');
});

test('menuImageSource passes digest-pinned RepoTags through as the source', () => {
  assert.equal(
    menuImageSource({ RepoTags: ['cgr.dev/chainguard/node@sha256:2a2df3a1f79cfe63317e3e6e2b7394cac434648fdfc82e4f702d0e6dc4d9a852'] }),
    'cgr.dev/chainguard/node@sha256:2a2df3a1f79cfe63317e3e6e2b7394cac434648fdfc82e4f702d0e6dc4d9a852',
  );
  // UI shape of the same image (name split on the last ':')
  assert.equal(
    menuImageSource({
      name: 'cgr.dev/chainguard/node@sha256',
      tag: '2a2df3a1f79cfe63317e3e6e2b7394cac434648fdfc82e4f702d0e6dc4d9a852',
    }),
    'cgr.dev/chainguard/node@sha256:2a2df3a1f79cfe63317e3e6e2b7394cac434648fdfc82e4f702d0e6dc4d9a852',
  );
});
