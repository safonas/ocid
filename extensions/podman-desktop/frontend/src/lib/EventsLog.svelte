<script lang="ts">
  import { Dropdown } from '@podman-desktop/ui-svelte';
  import type { DaemonEvent } from '../../../src/types';

  let { events }: { events: DaemonEvent[] } = $props();

  let filter = $state('all');

  const options = [
    { value: 'all', label: 'all' },
    { value: 'gossip', label: 'gossip' },
    { value: 'release_saved', label: 'release_saved' },
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
      case 'pruned':
        return 'pruned';
      case 'peer_change':
        return 'peer';
      default:
        return 'http';
    }
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
