<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    children,
    minWidth = '0'
  }: {
    children: Snippet;
    minWidth?: string;
  } = $props();
</script>

<div class="panel ui-data-panel table-panel" style={`--ui-table-min-width: ${minWidth}`}>
  {@render children()}
</div>

<style>
  :global(.ui-data-panel .ui-data-table) {
    min-width: var(--ui-table-min-width);
  }
</style>
