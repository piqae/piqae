<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    children,
    id,
    flush = false
  }: {
    children: Snippet;
    id?: string;
    flush?: boolean;
  } = $props();
</script>

<section class="panel" class:flush {id}>
  {@render children()}
</section>

<style>
  .panel:not(.flush) {
    padding: 16px;
  }
</style>
