<script lang="ts">
  import { Button } from '@podman-desktop/ui-svelte';
  import { sendAction } from '../api';
  import { publisherColor } from './format';
  import TimeAgo from './TimeAgo.svelte';
  import type { PeerInfo, Status } from '../../../src/types';

  let { peers, status }: { peers: PeerInfo[]; status?: Status } = $props();

  let ticket = $state('');

  /** A disconnected peer not heard from in over a week. */
  const WEEK_SECS = 7 * 86_400;
  function isStale(p: PeerInfo): boolean {
    return (
      !p.neighbor &&
      p.last_seen_secs !== null &&
      p.last_seen_secs !== undefined &&
      p.last_seen_secs > WEEK_SECS
    );
  }

  function follows(p: PeerInfo): boolean {
    return status?.follows.some(f => f.split(' ')[0] === p.id) ?? false;
  }

  // last_seen_secs is "seconds ago"; TimeAgo renders epoch seconds, so
  // anchor it to a ticking now (which also refreshes the stale greying).
  let now = $state(Date.now());
  $effect(() => {
    const timer = setInterval(() => (now = Date.now()), 15_000);
    return () => clearInterval(timer);
  });

  function connect(): void {
    const t = ticket.trim();
    if (t) sendAction({ kind: 'connect', ticket: t });
    ticket = '';
  }
</script>

<div class="bar">
  <textarea
    class="mono"
    rows="1"
    placeholder="paste a connection ticket…"
    bind:value={ticket}
  ></textarea>
  <Button onclick={connect}>Connect</Button>
</div>

{#if peers.length === 0}
  <p class="dim">
    No peers known yet. Ask a teammate for their ticket (Peers are also discovered
    automatically on the LAN via mDNS).
  </p>
{:else}
  <div class="table-wrap">
    <table style="min-width: 480px">
      <thead>
      <tr>
        <th>Peer</th>
        <th>Neighbor</th>
        <th>Known</th>
        <th>Last seen</th>
        <th></th>
      </tr>
    </thead>
    <tbody>
      {#each peers as p (p.id)}
        <tr class:stale={isStale(p)} title={isStale(p) ? 'last seen more than a week ago' : undefined}>
          <td class="mono" title={p.id}>
            <span class="pdot" style="background: {publisherColor(p.id)}"></span>{p.id.slice(0, 24)}…
          </td>
          <td>
            {#if p.neighbor}<span class="badge ok">connected</span>{/if}
          </td>
          <td>{#if p.known}<span class="badge">known</span>{/if}</td>
          <td>
            {#if p.last_seen_secs === null || p.last_seen_secs === undefined}
              <span class="dim">never</span>
            {:else}
              <TimeAgo ts={Math.floor(now / 1000) - p.last_seen_secs} />
            {/if}
          </td>
          <td class="actions">
            <Button type="secondary" padding="px-2 py-0.5" onclick={() => sendAction({ kind: 'sync', peer: p.id })}>Sync</Button>
            <Button
              type="secondary"
              padding="px-2 py-0.5"
              onclick={() =>
                sendAction(
                  follows(p)
                    ? { kind: 'unfollow', publisher: p.id }
                    : { kind: 'follow', publisher: p.id, mode: 'latest' },
                )}>
              {follows(p) ? 'Unfollow' : 'Follow'}
            </Button>
          </td>
        </tr>
      {/each}
    </tbody>
    </table>
  </div>
{/if}
