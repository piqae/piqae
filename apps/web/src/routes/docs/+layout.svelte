<script lang="ts">
  import DocsShell from '$lib/components/DocsShell.svelte';
  import type { Snippet } from 'svelte';
  let { children }: { children: Snippet } = $props();
</script>

<DocsShell>{@render children()}</DocsShell>
