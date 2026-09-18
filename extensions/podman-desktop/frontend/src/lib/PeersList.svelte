<script lang="ts">
  import { Button } from '@podman-desktop/ui-svelte';
  import { sendAction } from '../api';
  import type { PeerInfo } from '../../../src/types';

  let { peers }: { peers: PeerInfo[] } = $props();

  let ticket = $state('');

  function fmtSeen(secs?: number | null): string {
    return secs === undefined || secs === null ? '-' : `${secs}s ago`;
  }

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
  <table>
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
        <tr>
          <td class="mono" title={p.id}>{p.id.slice(0, 24)}…</td>
          <td>
            {#if p.neighbor}<span class="badge ok">connected</span>{/if}
          </td>
          <td>{#if p.known}<span class="badge">known</span>{/if}</td>
          <td>{fmtSeen(p.last_seen_secs)}</td>
          <td class="actions">
            <Button type="secondary" padding="px-2 py-0.5" onclick={() => sendAction({ kind: 'sync', peer: p.id })}>Sync</Button>
            <Button type="secondary" padding="px-2 py-0.5" onclick={() => sendAction({ kind: 'follow', publisher: p.id, mode: 'latest' })}>
              Follow
            </Button>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}
