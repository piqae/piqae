<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    title,
    description,
    id,
    actions
  }: {
    title: string;
    description?: string;
    id?: string;
    actions?: Snippet;
  } = $props();
</script>

<header>
  <div>
    <h2 {id}>{title}</h2>
    {#if description}<p>{description}</p>{/if}
  </div>
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</header>

<style>
  header {
    min-height: 52px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border-subtle);
  }

  h2 {
    margin: 0;
    font-size: var(--text-section);
    line-height: var(--text-section-line);
    font-weight: 560;
  }

  p {
    margin: 2px 0 0;
    color: var(--text-secondary);
    font-size: var(--text-meta);
    line-height: var(--text-meta-line);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  @media (max-width: 620px) {
    header {
      align-items: flex-start;
      flex-direction: column;
      gap: 10px;
    }
  }
</style>
