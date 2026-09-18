<script lang="ts">
  import { timeAgo } from './format';

  let { ts }: { ts: number } = $props();

  let now = $state(Date.now());
  $effect(() => {
    const timer = setInterval(() => (now = Date.now()), 15_000);
    return () => clearInterval(timer);
  });
</script>

<!-- ts is epoch seconds (daemon DTO) -->
<span title={new Date(ts * 1000).toLocaleString()}>{timeAgo(ts, now)}</span>
