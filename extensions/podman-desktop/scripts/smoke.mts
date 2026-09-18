// Functional smoke test for the extension backend against a live ocid daemon:
// exercises exactly the code paths the webview drives (OcidClient + SseWatcher)
// — every /_ocid endpoint the dashboard uses, plus the SSE event stream.
//
// Run via `just ext-smoke` (starts a throwaway daemon on 127.0.0.1:5070).

import { generateKeyPairSync } from 'node:crypto';
import assert from 'node:assert/strict';
import { OcidClient } from '../src/ocid-client.ts';
import { EventRing, SseWatcher } from '../src/sse.ts';

const BASE = process.env.OCID_BASE ?? 'http://127.0.0.1:5070';
const client = new OcidClient(BASE);

// A valid Ed25519 public key we do not know: fine for policy rules.
const stranger = generateKeyPairSync('ed25519')
  .publicKey.export({ type: 'spki', format: 'der' })
  .subarray(-32)
  .toString('hex');

async function waitFor(cond: () => boolean, what: string, ms = 10_000): Promise<void> {
  const deadline = Date.now() + ms;
  while (!cond()) {
    if (Date.now() > deadline) throw new Error(`timeout waiting for ${what}`);
    await new Promise(r => setTimeout(r, 200));
  }
}

// --- status, releases, peers -------------------------------------------------

const status = await client.status();
assert.equal(typeof status.id, 'string');
assert.match(status.did, /^did:key:/);
assert.ok(status.ticket.length > 0);
assert.equal(status.releases, 0);
console.log('ok  status', status.did);

assert.deepEqual(await client.releases(), []);
assert.deepEqual(await client.peers(), []);
console.log('ok  empty releases/peers');

// --- policy endpoints --------------------------------------------------------

const ring = new EventRing();
const sse = new SseWatcher(BASE, ring, () => {});
sse.start();

let r = await client.follow(stranger, 'latest');
assert.equal(r.changed, true);
assert.equal(r.reference, stranger);
assert.deepEqual((await client.status()).follows, [`${stranger} [latest]`]);
r = await client.follow(stranger, 'latest');
assert.equal(r.changed, false, 'idempotent follow');
console.log('ok  follow');

r = await client.unfollow(stranger);
assert.equal(r.changed, true);
assert.deepEqual((await client.status()).follows, []);
r = await client.unfollow(stranger);
assert.equal(r.changed, false, 'unfollow of nothing');
console.log('ok  unfollow');

const img = `${stranger}/webapp`;
r = await client.seed(img, 'last:3');
assert.equal(r.changed, true);
assert.equal(r.reference, img);
assert.deepEqual((await client.status()).seeds, [`${img} [last:3]`]);
console.log('ok  seed');

const tagged = `${img}:1.0`;
r = await client.pin(tagged);
assert.equal(r.changed, true);
assert.equal(r.reference, tagged);
assert.deepEqual((await client.status()).pins, [tagged]);
await assert.rejects(() => client.pin(img), /pin needs a tag/);
r = await client.unpin(tagged);
assert.equal(r.changed, true);
assert.deepEqual((await client.status()).pins, []);
console.log('ok  pin/unpin');

r = await client.unseed(img);
assert.equal(r.changed, true);
assert.deepEqual((await client.status()).seeds, []);
console.log('ok  unseed');

// --- error mapping -----------------------------------------------------------

await assert.rejects(
  () => client.seed(`${status.id}/self`, 'latest'),
  /is published by this node; it is always seeded/,
);
await assert.rejects(() => client.pull(`${stranger}/nope:1`), /not found/);
console.log('ok  error surface');

// --- misc endpoints ----------------------------------------------------------

// The synced peer count depends on the environment (mDNS finds daemons
// running on this host/LAN); only failures would be a bug.
const sync = await client.sync();
assert.deepEqual(sync.failed, []);
const gc = await client.gc(true, false);
assert.equal(gc.dry_run, true);
const rm = await client.rm(tagged, false);
assert.deepEqual(rm.removed, []);
console.log('ok  sync/gc/rm');

// --- SSE ---------------------------------------------------------------------

// With a listener attached, every control request above emitted an
// http_request event; poll actions generate more.
await waitFor(() => ring.snapshot().some(e => e.type === 'http_request'), 'SSE events');
sse.stop();
console.log('ok  SSE stream', ring.snapshot().length, 'events buffered');

console.log('\nsmoke: all checks passed');
