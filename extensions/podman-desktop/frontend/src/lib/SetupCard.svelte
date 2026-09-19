<script lang="ts">
  import { Button } from '@podman-desktop/ui-svelte';
  import { sendAction } from '../api';
  import type { SetupState } from '../../../src/types';

  let { setup, daemon }: { setup?: SetupState; daemon: boolean } = $props();

  const INSTALL = 'https://github.com/safonas/ocid/releases';
</script>

{#if setup && !daemon}
  <div class="card setup">
    <h2>Set up ocid</h2>
    {#if setup.ocidPath}
      <p>
        The ocid daemon is not running. It serves the registry and control API on
        <code class="mono">{setup.registryHost}</code> and keeps running when Podman Desktop closes.
      </p>
      <div class="row">
        <Button type="primary" onclick={() => sendAction({ kind: 'start-daemon' })}>
          Start ocid daemon
        </Button>
        <span class="dim">from <code class="mono">{setup.ocidPath}</code></span>
      </div>
    {:else}
      <p>Install the ocid daemon on this machine, then come back to start it:</p>
      <pre class="mono">brew install safonas/ocid/ocid</pre>
      <p class="dim">
        or grab a bundle (<code class="mono">ocid</code>, <code class="mono">ocictl</code>,
        <code class="mono">ocitop</code>) from the
        <a href={INSTALL} target="_blank" rel="noreferrer">releases page</a>.
      </p>
    {/if}
  </div>
{:else if setup && daemon && !setup.registered}
  <div class="card setup">
    <h2>Register the ocid registry with podman</h2>
    {#if setup.platform === 'linux'}
      <p>
        Adds <code class="mono">{setup.registryHost}</code> as an insecure (plain http, loopback)
        registry via <code class="mono">~/.config/containers/registries.conf.d/100-ocid.conf</code>,
        so pushes and pulls work without TLS bypasses. Rootless podman picks it up immediately;
        rootful podman needs the same file under
        <code class="mono">/etc/containers/registries.conf.d/</code>.
      </p>
      <Button type="primary" onclick={() => sendAction({ kind: 'register-registry' })}>
        Register {setup.registryHost}
      </Button>
    {:else}
      <p>
        Automatic registration is not available on this platform yet; pushes and pulls use a
        <code class="mono">--tls-verify=false</code> bypass in the meantime.
      </p>
    {/if}
  </div>
{/if}
