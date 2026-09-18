<script lang="ts">
  import { Button } from '@podman-desktop/ui-svelte';
  import QRCode from 'qrcode';
  import { sendAction } from '../api';

  let { ticket }: { ticket: string } = $props();

  let qrSvg = $state('');
  let copied = $state(false);

  $effect(() => {
    if (!ticket) {
      qrSvg = '';
      return;
    }
    QRCode.toString(ticket, { type: 'svg', margin: 1, width: 180 })
      .then(svg => (qrSvg = svg))
      .catch(() => (qrSvg = ''));
  });

  async function copy(): Promise<void> {
    await navigator.clipboard.writeText(ticket);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }
</script>

<div class="ticket">
  <div class="left">
    <h3>Connection ticket</h3>
    <p class="dim">
      Share this with teammates: they paste it on their Peers tab (or scan the QR on a
      LAN machine), then follow your publisher to replicate your images.
    </p>
    <textarea class="mono" readonly rows="3" value={ticket}></textarea>
    <Button onclick={() => void copy()}>
      {copied ? 'Copied!' : 'Copy ticket'}
    </Button>
  </div>
  <div class="qr">
    {#if qrSvg}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -- generated locally from the ticket -->
      {@html qrSvg}
    {/if}
  </div>
</div>
