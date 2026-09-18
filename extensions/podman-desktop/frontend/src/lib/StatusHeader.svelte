<script lang="ts">
  import type { Status } from '../../../src/types';

  let { status }: { status?: Status } = $props();

  function fmtUptime(secs: number): string {
    if (secs < 60) return `${secs}s`;
    if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
    if (secs < 86400) return `${Math.floor(secs / 3600)}h ${Math.floor((secs % 3600) / 60)}m`;
    return `${Math.floor(secs / 86400)}d ${Math.floor((secs % 86400) / 3600)}h`;
  }
</script>

{#if status}
  <div class="cards">
    <div class="card">
      <div class="label">Publisher</div>
      <div class="value mono" title={status.did}>{status.did.replace('did:key:', '').slice(0, 24)}…</div>
    </div>
    <div class="card">
      <div class="label">Peers</div>
      <div class="value">
        {status.neighbors.length} connected / {status.known_peers} known
      </div>
    </div>
    <div class="card">
      <div class="label">Releases</div>
      <div class="value">{status.releases}</div>
    </div>
    <div class="card">
      <div class="label">Policy</div>
      <div class="value">
        {status.follows.length} follow · {status.seeds.length} seed · {status.pins.length} pin
      </div>
    </div>
    <div class="card">
      <div class="label">Registry</div>
      <div class="value mono">{status.registry}</div>
    </div>
    <div class="card">
      <div class="label">Uptime</div>
      <div class="value">{fmtUptime(status.uptime_secs)}</div>
    </div>
    <div class="card">
      <div class="label">Version</div>
      <div class="value mono">{status.version}</div>
    </div>
  </div>
{/if}
