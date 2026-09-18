<script lang="ts">
  import { Dropdown } from '@podman-desktop/ui-svelte';
  import type { DaemonEvent } from '../../../src/types';

  let { events }: { events: DaemonEvent[] } = $props();

  let filter = $state('all');

  const options = [
    { value: 'all', label: 'all' },
    { value: 'gossip', label: 'gossip' },
    { value: 'release_saved', label: 'release_saved' },
    { value: 'pull_progress', label: 'pull_progress' },
    { value: 'fetch_failed', label: 'fetch_failed' },
    { value: 'pruned', label: 'pruned' },
    { value: 'peer_change', label: 'peer_change' },
    { value: 'http_request', label: 'http_request' },
  ];

  const shown = $derived(
    (filter === 'all' ? [...events] : events.filter(e => e.type === filter)).reverse(),
  );

  function describe(e: DaemonEvent): string {
    switch (e.type) {
      case 'gossip':
        return `${e.outbound ? 'announced' : 'received gossip for'} ${e.name}:${e.tag} (${e.publisher.slice(0, 12)}…)`;
      case 'pruned':
        return `pruned ${e.name}:${e.tag} — ${e.reason}`;
      case 'release_saved':
        return `saved ${e.name}:${e.tag} (${e.blobs} blobs)`;
      case 'pull_progress':
        return `pulling ${e.name}:${e.tag} — ${e.blobs_done}/${e.blobs_total} blobs (${fmtBytes(e.bytes_done)} / ${fmtBytes(e.bytes_total)})`;
      case 'fetch_failed':
        return `pull failed: ${e.name}:${e.tag} — ${e.error}`;
      case 'peer_change':
        return `peer ${e.connected ? 'connected' : 'disconnected'} (${e.id.slice(0, 12)}…)`;
      case 'http_request':
        return `${e.method} ${e.path} → ${e.status}`;
    }
  }

  function cls(e: DaemonEvent): string {
    switch (e.type) {
      case 'gossip':
        return 'gossip';
      case 'release_saved':
        return 'saved';
      case 'pull_progress':
        return 'pull';
      case 'fetch_failed':
        return 'pruned';
      case 'pruned':
        return 'pruned';
      case 'peer_change':
        return 'peer';
      default:
        return 'http';
    }
  }

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KiB`;
    if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MiB`;
    return `${(n / 1024 ** 3).toFixed(1)} GiB`;
  }
</script>

<div class="bar">
  <Dropdown ariaLabel="Filter events" bind:value={filter} {options} />
</div>

<div class="log">
  {#each shown as e, i (shown.length - i)}
    <div class="line {cls(e)}">
      <span class="tag">{e.type}</span>
      <span>{describe(e)}</span>
    </div>
  {/each}
  {#if shown.length === 0}
    <p class="dim">No events yet — they arrive live from the daemon (and via the 2s poll otherwise).</p>
  {/if}
</div>
