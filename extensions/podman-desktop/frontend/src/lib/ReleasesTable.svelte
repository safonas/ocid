<script lang="ts">
  import { Button, Input } from '@podman-desktop/ui-svelte';
  import { sendAction } from '../api';
  import { fmtBytes, publisherColor, shortId } from './format';
  import TimeAgo from './TimeAgo.svelte';
  import type { ReleaseInfo, Status } from '../../../src/types';

  let { releases, status }: { releases: ReleaseInfo[]; status?: Status } = $props();

  let pullRef = $state('');
  let copiedRun = $state('');

  // Coarse display-only classification from the Status rule strings (the
  // daemon stays the single policy enforcer; this never decides retention).
  // Seeds/follows entries are "<rule> [mode]"; pins are bare "<pub/name>:<tag>".
  function keptBy(r: ReleaseInfo): string {
    if (r.mine) return 'own';
    const ref = `${r.publisher}/${r.name}:${r.tag}`;
    if (status?.pins.includes(ref)) return 'pin';
    if (status?.follows.some(f => f.split(' ')[0] === r.publisher)) return 'follow';
    if (
      status?.seeds.some(s => {
        const rule = s.split(' ')[0];
        return rule === `${r.publisher}/${r.name}` || rule === ref;
      })
    ) {
      return 'seed';
    }
    return 'cache';
  }

  function isSeeded(r: ReleaseInfo): boolean {
    const kept = keptBy(r);
    return kept === 'seed' || kept === 'pin' || kept === 'follow';
  }

  function ref(r: ReleaseInfo, tagged = true): string {
    return tagged ? `${r.publisher}/${r.name}:${r.tag}` : `${r.publisher}/${r.name}`;
  }

  function runCmd(r: ReleaseInfo): string {
    const reg = status?.registry ?? '127.0.0.1:5050';
    return `podman run ${reg}/${r.publisher}/${r.name}:${r.tag}`;
  }

  async function copyRun(r: ReleaseInfo): Promise<void> {
    await navigator.clipboard.writeText(runCmd(r));
    copiedRun = `${r.publisher}/${r.name}:${r.tag}`;
    setTimeout(() => (copiedRun = ''), 1500);
  }

  function pull(): void {
    const reference = pullRef.trim();
    if (reference) sendAction({ kind: 'pull', reference });
    pullRef = '';
  }
</script>

<div class="bar">
  <Input
    class="mono"
    placeholder="pull <publisher>/<name>[:<tag>]"
    aria-label="Reference to pull"
    bind:value={pullRef}
    onkeypress={e => e.key === 'Enter' && pull()}
  />
  <Button onclick={pull}>Pull</Button>
</div>

{#if releases.length === 0}
  <p class="dim">
    No releases yet. Push one: <code class="mono">podman push 127.0.0.1:5050/&lt;name&gt;:&lt;tag&gt;</code>
  </p>
{:else}
  <table>
    <thead>
      <tr>
        <th>Publisher</th>
        <th>Image</th>
        <th>Tag</th>
        <th>Updated</th>
        <th>Digest</th>
        <th>Size</th>
        <th>State</th>
        <th>Kept by</th>
        <th></th>
      </tr>
    </thead>
    <tbody>
      {#each releases as r (r.publisher + r.name + r.tag)}
        <tr>
          <td class="mono" title={r.publisher}>
            <span class="pdot" style="background: {publisherColor(r.publisher)}"></span>{r.mine
              ? '(me)'
              : shortId(r.publisher)}
          </td>
          <td>{r.name}</td>
          <td class="mono">{r.tag}</td>
          <td><TimeAgo ts={r.timestamp} /></td>
          <td class="mono" title={r.manifest_digest}>{r.manifest_digest.replace('sha256:', '').slice(0, 12)}…</td>
          <td title="{r.blobs} blob(s)">{fmtBytes(r.size)}</td>
          <td>
            <span class="badge" class:ok={r.complete} class:warn={!r.complete}>
              {r.complete ? 'complete' : 'partial'}
            </span>
          </td>
          <td>
            <span class="badge" class:ok={keptBy(r) !== 'cache'}>{keptBy(r)}</span>
          </td>
          <td class="actions">
            <Button
              type="secondary"
              padding="px-2 py-0.5"
              title="Copy a ready-to-paste podman run command"
              onclick={() => void copyRun(r)}
            >
              {copiedRun === `${r.publisher}/${r.name}:${r.tag}` ? 'Copied!' : 'Run cmd'}
            </Button>
            {#if !r.mine}
              {#if isSeeded(r)}
                <Button type="secondary" padding="px-2 py-0.5" onclick={() => sendAction({ kind: 'unseed', reference: ref(r, false) })}>
                  Unseed
                </Button>
              {:else}
                <Button type="secondary" padding="px-2 py-0.5" onclick={() => sendAction({ kind: 'seed', reference: ref(r, false), mode: 'latest' })}>
                  Seed
                </Button>
              {/if}
              <Button type="secondary" padding="px-2 py-0.5" onclick={() => sendAction({ kind: 'pin', reference: ref(r) })}>Pin</Button>
              <Button type="secondary" padding="px-2 py-0.5" onclick={() => sendAction({ kind: 'unpin', reference: ref(r) })}>Unpin</Button>
              <Button type="secondary" padding="px-2 py-0.5" onclick={() => sendAction({ kind: 'follow', publisher: r.publisher, mode: 'latest' })}>
                Follow
              </Button>
            {/if}
            <Button type="danger" padding="px-2 py-0.5" onclick={() => sendAction({ kind: 'rm', reference: ref(r), allTags: false })}>
              Remove
            </Button>
          </td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}
