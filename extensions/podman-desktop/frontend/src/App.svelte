<script lang="ts">
  import { Button } from '@podman-desktop/ui-svelte';
  import { onState, sendAction } from './api';
  import type { StateSnapshot } from '../../src/types';
  import StatusHeader from './lib/StatusHeader.svelte';
  import TicketCard from './lib/TicketCard.svelte';
  import Transfers from './lib/Transfers.svelte';
  import ReleasesTable from './lib/ReleasesTable.svelte';
  import PeersList from './lib/PeersList.svelte';
  import EventsLog from './lib/EventsLog.svelte';
  import SetupCard from './lib/SetupCard.svelte';

  let snap: StateSnapshot = $state({ daemon: false, events: [], transfers: [] });
  onState(s => {
    snap = s;
  });

  let tab: 'overview' | 'releases' | 'peers' | 'events' = $state('overview');

  const tabs = $derived([
    { id: 'overview', label: 'Overview' },
    { id: 'releases', label: `Releases${snap.releases ? ` (${snap.releases.length})` : ''}` },
    { id: 'peers', label: `Peers${snap.peers ? ` (${snap.peers.length})` : ''}` },
    { id: 'events', label: 'Events' },
  ] as const);
</script>

<div class="topbar">
  <h1>ocid</h1>
  {#if snap.daemon}
    <span class="badge ok">daemon up</span>
    <Button type="secondary" onclick={() => sendAction({ kind: 'sync' })}>Sync</Button>
    <Button type="secondary" onclick={() => sendAction({ kind: 'gc', dryRun: true, force: false })}>
      GC (dry-run)
    </Button>
    <Button type="secondary" onclick={() => sendAction({ kind: 'gc', dryRun: false, force: false })}>
      Collect
    </Button>
    <Button type="secondary" onclick={() => sendAction({ kind: 'announce' })}>Announce</Button>
  {:else}
    <span class="badge err">daemon down</span>
  {/if}
</div>

{#if snap.error}
  <div class="error-bar">{snap.error}</div>
{/if}

{#if !snap.daemon}
  <SetupCard setup={snap.setup} daemon={false} />
  <div class="empty">
    <p class="dim">
      The dashboard connects automatically once the daemon serves
      <code class="mono">127.0.0.1:5050</code>.
    </p>
  </div>
{:else}
  {#if snap.setup && !snap.setup.registered}
    <SetupCard setup={snap.setup} daemon={true} />
  {/if}
  <nav class="tabs">
    {#each tabs as t (t.id)}
      <Button type="tab" selected={tab === t.id} onclick={() => (tab = t.id)}>{t.label}</Button>
    {/each}
  </nav>

  {#if snap.transfers.length}
    <Transfers transfers={snap.transfers} />
  {/if}

  {#if tab === 'overview'}
    <StatusHeader status={snap.status} />
    <TicketCard ticket={snap.status?.ticket ?? ''} />
  {:else if tab === 'releases'}
    <ReleasesTable releases={snap.releases ?? []} status={snap.status} />
  {:else if tab === 'peers'}
    <PeersList peers={snap.peers ?? []} />
  {:else}
    <EventsLog events={snap.events} />
  {/if}
{/if}
