<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    children,
    meta
  }: {
    children: Snippet;
    meta?: string;
  } = $props();
</script>

<div class="ui-toolbar">
  {@render children()}
  {#if meta}<span class="ui-toolbar__meta">{meta}</span>{/if}
</div>
