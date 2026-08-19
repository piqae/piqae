<script lang="ts">
  import AppShell from '$lib/components/AppShell.svelte';
  import type { Snippet } from 'svelte';
  import type { LayoutData } from './$types';

  let { data, children }: { data: LayoutData; children: Snippet } = $props();
</script>

<AppShell
  mode={data.dashboardMode ?? 'live'}
  meta={data.meta}
  viewer={data.viewer}
  workspaces={data.workspaces}
>{@render children()}</AppShell>
