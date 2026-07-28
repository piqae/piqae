<script lang="ts">
  import AppShell from '$lib/components/AppShell.svelte';
  import type { Snippet } from 'svelte';

  let { data, children }: { data: App.PageData; children: Snippet } = $props();
</script>

<AppShell mode={data.dashboardMode ?? 'live'}>{@render children()}</AppShell>
