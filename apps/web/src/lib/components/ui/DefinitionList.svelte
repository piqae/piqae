<script lang="ts">
  import type { Snippet } from 'svelte';

  export type Definition = {
    term: string;
    value?: string | number | null;
    mono?: boolean;
    render?: Snippet;
  };

  let {
    items,
    columns = 1
  }: {
    items: Definition[];
    columns?: 1 | 2;
  } = $props();
</script>

<dl style={`--definition-columns: ${columns}`}>
  {#each items as item}
    <div>
      <dt>{item.term}</dt>
      <dd class:mono={item.mono}>
        {#if item.render}
          {@render item.render()}
        {:else}
          {item.value ?? '—'}
        {/if}
      </dd>
    </div>
  {/each}
</dl>

<style>
  dl {
    display: grid;
    grid-template-columns: repeat(var(--definition-columns), minmax(0, 1fr));
    margin: 0;
    column-gap: 24px;
  }

  div {
    min-height: var(--row-normal);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    border-bottom: 1px solid var(--border-subtle);
  }

  dt {
    flex: 0 0 auto;
    color: var(--text-tertiary);
    font-size: var(--text-compact);
  }

  dd {
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--text-secondary);
    font-size: var(--text-compact);
    text-align: right;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  dd:not(.mono) {
    text-transform: capitalize;
  }

  dd :global(a:hover) {
    color: var(--text-primary);
  }

  @media (max-width: 620px) {
    dl {
      grid-template-columns: 1fr;
    }
  }
</style>
