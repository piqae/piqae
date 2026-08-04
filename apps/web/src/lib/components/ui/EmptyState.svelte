<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    message,
    compact = false,
    action
  }: {
    message: string;
    compact?: boolean;
    action?: Snippet;
  } = $props();
</script>

<div class="empty-state" class:compact>
  <div>
    <p>{message}</p>
    {#if action}<div class="action">{@render action()}</div>{/if}
  </div>
</div>

<style>
  .compact {
    min-height: 120px;
  }

  p {
    margin: 0;
    color: var(--text-secondary);
  }

  .action {
    margin-top: 12px;
  }
</style>
