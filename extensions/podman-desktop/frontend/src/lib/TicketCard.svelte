<script lang="ts">
  import { Button } from '@podman-desktop/ui-svelte';
  import QRCode from 'qrcode';
  import { sendAction } from '../api';

  let { ticket }: { ticket: string } = $props();

  let qrSvg = $state('');

  $effect(() => {
    if (!ticket) {
      qrSvg = '';
      return;
    }
    QRCode.toString(ticket, { type: 'svg', margin: 1, width: 180 })
      .then(svg => (qrSvg = svg))
      .catch(() => (qrSvg = ''));
  });

  function copy(): void {
    // navigator.clipboard is unavailable in PD webviews; the backend
    // performs the copy through the extension API.
    sendAction({ kind: 'copy', text: ticket });
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
    <Button onclick={copy}>Copy ticket</Button>
  </div>
  <div class="qr">
    {#if qrSvg}
      <!-- eslint-disable-next-line svelte/no-at-html-tags -- generated locally from the ticket -->
      {@html qrSvg}
    {/if}
  </div>
</div>
