// Unit tests for the extension's transfer view model (no daemon needed):
// run via `just ext-test` (node --experimental-strip-types --test).

import test from 'node:test';
import assert from 'node:assert/strict';
import { TransferTracker } from '../src/transfers.ts';
import type { DaemonEvent } from '../src/types.ts';

const PUB = 'aa'.repeat(32);

function progress(
  blobsDone: number,
  blobsTotal: number,
  bytesDone: number,
  bytesTotal: number,
): DaemonEvent {
  return {
    type: 'pull_progress',
    publisher: PUB,
    name: 'webapp',
    tag: 'v1',
    blobs_done: blobsDone,
    blobs_total: blobsTotal,
    bytes_done: bytesDone,
    bytes_total: bytesTotal,
  };
}

const saved: DaemonEvent = {
  type: 'release_saved',
  publisher: PUB,
  name: 'webapp',
  tag: 'v1',
  blobs: 2,
};

const failed: DaemonEvent = {
  type: 'fetch_failed',
  publisher: PUB,
  name: 'webapp',
  tag: 'v1',
  error: 'no providers for blob sha256:dead',
};

test('progress events create and advance a transfer', () => {
  const t = new TransferTracker();
  t.onEvent(progress(0, 2, 0, 1000), 1_000);
  let rows = t.list();
  assert.equal(rows.length, 1);
  assert.equal(rows[0].key, `${PUB}/webapp:v1`);
  assert.equal(rows[0].state, 'active');
  assert.equal(rows[0].bytesTotal, 1000);

  t.onEvent(progress(1, 2, 400, 1000), 2_000);
  rows = t.list();
  assert.equal(rows[0].bytesDone, 400);
  assert.equal(rows[0].blobsDone, 1);
  // first measurable delta: rate = instant rate
  assert.equal(rows[0].bytesPerSec, 400);
});

test('rate is smoothed, not recomputed on bursts', () => {
  const t = new TransferTracker();
  t.onEvent(progress(0, 3, 0, 3000), 0);
  t.onEvent(progress(1, 3, 1000, 3000), 1_000); // instant 1000 B/s
  // burst 100ms later: below RATE_MIN_DT_MS, rate must stay untouched
  t.onEvent(progress(2, 3, 2000, 3000), 1_100);
  assert.equal(t.list()[0].bytesPerSec, 1000);
  // next measurable step (1s, 1000B): 0.3*1000 + 0.7*1000 = 1000
  t.onEvent(progress(3, 3, 3000, 3000), 2_100);
  assert.equal(t.list()[0].bytesPerSec, 1000);
});

test('release_saved completes the transfer and it lingers then drops', () => {
  const t = new TransferTracker();
  t.onEvent(progress(1, 2, 100, 500), 1_000);
  t.onEvent(saved, 1_100);
  let rows = t.list();
  assert.equal(rows[0].state, 'done');
  assert.equal(rows[0].bytesDone, 500);
  assert.equal(rows[0].blobsDone, 2);
  assert.equal(rows[0].bytesPerSec, 0);

  t.prune(1_100 + 5_000); // exactly at the linger limit: still there
  assert.equal(t.list().length, 1);
  t.prune(1_100 + 5_001); // past it: gone
  assert.equal(t.list().length, 0);
});

test('fetch_failed marks the transfer failed with the error', () => {
  const t = new TransferTracker();
  t.onEvent(progress(1, 2, 100, 500), 1_000);
  t.onEvent(failed, 1_200);
  const rows = t.list();
  assert.equal(rows[0].state, 'failed');
  assert.equal(rows[0].error, 'no providers for blob sha256:dead');
});

test('terminal events without an active transfer are ignored', () => {
  const t = new TransferTracker();
  t.onEvent(saved, 1_000);
  t.onEvent(failed, 1_100);
  assert.equal(t.list().length, 0);
});

test('stale active transfers are dropped', () => {
  const t = new TransferTracker();
  t.onEvent(progress(0, 2, 0, 1000), 1_000);
  t.prune(1_000 + 60_000); // exactly at the stale limit: still there
  assert.equal(t.list().length, 1);
  t.prune(1_000 + 60_001); // past it: gone
  assert.equal(t.list().length, 0);
});

test('list is ordered by most recent update', () => {
  const t = new TransferTracker();
  t.onEvent(
    { type: 'pull_progress', publisher: 'b'.repeat(64), name: 'x', tag: '1', blobs_done: 0, blobs_total: 1, bytes_done: 0, bytes_total: 10 },
    1_000,
  );
  t.onEvent(progress(0, 2, 0, 1000), 2_000);
  const rows = t.list();
  assert.equal(rows.length, 2);
  assert.equal(rows[0].name, 'webapp'); // updated later
  assert.equal(rows[1].name, 'x');
});
