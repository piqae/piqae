<script lang="ts">
  import '../app.css';
  import '../marketing.css';
  import { beforeNavigate } from '$app/navigation';
  import { updated } from '$app/state';
  import type { Snippet } from 'svelte';

  let { children }: { children: Snippet } = $props();

  beforeNavigate(({ to, willUnload }) => {
    if (updated.current && !willUnload && to?.url) {
      window.location.assign(to.url.href);
    }
  });
</script>

{@render children()}
