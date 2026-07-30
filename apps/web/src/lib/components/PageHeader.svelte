<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    eyebrow,
    title,
    description,
    actions
  }: {
    eyebrow?: string;
    title: string;
    description?: string;
    actions?: Snippet;
  } = $props();
</script>

<header>
  <div>
    {#if eyebrow}<div class="eyebrow">{eyebrow}</div>{/if}
    <h1>{title}</h1>
    {#if description}<p>{description}</p>{/if}
  </div>
  {#if actions}
    <div class="actions">{@render actions()}</div>
  {/if}
</header>

<style>
  header {
    min-height: 72px;
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 24px;
    padding: 5px 0 16px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .eyebrow {
    margin-bottom: 2px;
    color: var(--text-tertiary);
    font-size: 11px;
    font-weight: 500;
    letter-spacing: 0.025em;
    text-transform: uppercase;
  }

  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 18px;
    line-height: 26px;
    font-weight: 570;
    letter-spacing: -0.027em;
  }

  p {
    margin: 3px 0 0;
    color: var(--text-secondary);
    font-size: 12px;
    line-height: 18px;
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 3px;
  }

  @media (max-width: 620px) {
    header {
      align-items: stretch;
      flex-direction: column;
      gap: 12px;
    }
  }
</style>
