<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    label,
    hint,
    children
  }: {
    label: string;
    hint?: string;
    children: Snippet;
  } = $props();
</script>

<label class="field">
  <span>{label}</span>
  {@render children()}
  {#if hint}<small>{hint}</small>{/if}
</label>

<style>
  small {
    color: var(--text-tertiary);
    font-size: var(--text-meta);
    line-height: var(--text-meta-line);
  }
</style>
