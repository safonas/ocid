<script lang="ts">
  import { fmtBytes, publisherColor, shortId } from './format';
  import type { TransferState } from '../../../src/types';

  let { transfers }: { transfers: TransferState[] } = $props();

  function pct(t: TransferState): number {
    return t.bytesTotal > 0 ? Math.min(100, (t.bytesDone / t.bytesTotal) * 100) : 0;
  }
</script>

<div class="transfers">
  {#each transfers as t (t.key)}
    <div class="transfer {t.state}">
      <span class="pdot" style="background: {publisherColor(t.publisher)}" title={t.publisher}></span>
      <span class="tref mono" title={t.key}>{t.name}:{t.tag}</span>
      <span class="tfrom">{shortId(t.publisher)}</span>
      <div
        class="tbar"
        role="progressbar"
        aria-label={`pulling ${t.key}`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(pct(t))}
      >
        <div class="tfill" style="width: {pct(t)}%"></div>
      </div>
      <span class="tmeta">
        {t.blobsDone}/{t.blobsTotal} blobs · {fmtBytes(t.bytesDone)} / {fmtBytes(t.bytesTotal)}
      </span>
      {#if t.state === 'active' && t.bytesPerSec > 1}
        <span class="tspeed">{fmtBytes(t.bytesPerSec)}/s</span>
      {:else if t.state === 'done'}
        <span class="tok">done</span>
      {:else if t.state === 'failed'}
        <span class="terr" title={t.error}>failed</span>
      {/if}
    </div>
  {/each}
</div>
