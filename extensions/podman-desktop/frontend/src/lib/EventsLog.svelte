<script lang="ts">
  import { Dropdown } from '@podman-desktop/ui-svelte';
  import { clock, fmtBytes, publisherColor } from './format';
  import type { TimedEvent } from '../../../src/types';

  let { events }: { events: TimedEvent[] } = $props();

  let filter = $state('all');
  let logEl: HTMLElement | undefined;

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

  // Follow the newest line unless the user has scrolled away from the bottom.
  $effect(() => {
    shown.length;
    if (logEl && logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight < 80) {
      logEl.scrollTop = logEl.scrollHeight;
    }
  });

  function describe(e: TimedEvent): string {
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

  function cls(e: TimedEvent): string {
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

  function icon(e: TimedEvent): string {
    switch (e.type) {
      case 'gossip':
        return e.outbound ? '→' : '←';
      case 'release_saved':
        return '✓';
      case 'pull_progress':
        return '↓';
      case 'fetch_failed':
        return '✕';
      case 'pruned':
        return '✂';
      case 'peer_change':
        return e.connected ? '⇄' : '⇥';
      case 'http_request':
        return '·';
    }
  }
</script>

<div class="bar">
  <Dropdown ariaLabel="Filter events" bind:value={filter} {options} />
</div>

<div class="log" bind:this={logEl}>
  {#each shown as e, i (shown.length - i)}
    <div class="line {cls(e)}">
      <span class="time mono">{clock(e.receivedAt)}</span>
      <span class="tag">{icon(e)} {e.type}</span>
      {#if 'publisher' in e}
        <span class="pdot" style="background: {publisherColor(e.publisher)}" title={e.publisher}></span>
      {/if}
      <span class="msg">{describe(e)}</span>
    </div>
  {/each}
  {#if shown.length === 0}
    <p class="dim">No events yet — they arrive live from the daemon (and via the 2s poll otherwise).</p>
  {/if}
</div>
