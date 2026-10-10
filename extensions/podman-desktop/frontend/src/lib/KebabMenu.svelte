<script lang="ts">
  import { Button } from '@podman-desktop/ui-svelte';

  interface Item {
    label: string;
    onclick: () => void;
  }

  let { items }: { items: Item[] } = $props();

  let open = $state(false);
  let anchor: HTMLElement | undefined;
  let pos = $state({ top: 0, left: 0 });

  // Fixed positioning escapes the table's overflow clipping; align the
  // menu's right edge with the button and flip up near the pane bottom.
  const MENU_W = 190;
  function toggle(): void {
    if (open) {
      open = false;
      return;
    }
    const r = anchor?.getBoundingClientRect();
    if (r) {
      const height = items.length * 30 + 8;
      pos.left = Math.max(8, Math.min(r.right - MENU_W, window.innerWidth - MENU_W - 8));
      pos.top =
        window.innerHeight - r.bottom < height + 8
          ? Math.max(8, r.top - height - 4)
          : r.bottom + 4;
    }
    open = true;
  }
</script>

<svelte:window
  on:click={e => open && anchor && !anchor.contains(e.target as Node) && (open = false)}
  on:keydown={e => e.key === 'Escape' && (open = false)}
/>

<span bind:this={anchor} class="kebab">
  <Button type="secondary" padding="px-2 py-0.5" aria-label="More actions" onclick={toggle}>
    &#8942;
  </Button>
  {#if open}
    <div class="menu" style="top:{pos.top}px; left:{pos.left}px" role="menu">
      {#each items as it (it.label)}
        <button
          type="button"
          role="menuitem"
          onclick={() => {
            open = false;
            it.onclick();
          }}>
          {it.label}
        </button>
      {/each}
    </div>
  {/if}
</span>
