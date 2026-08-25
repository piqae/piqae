<script lang="ts">
  import type { Snippet } from 'svelte';

  let {
    label,
    value,
    total,
    detail,
    href,
    title,
    tone = 'neutral'
  }: {
    label: string;
    value: string | number;
    total?: string | number;
    /** Plain supporting text, or a snippet when the detail needs emphasis. */
    detail?: string | Snippet;
    href?: string;
    title?: string;
    /**
     * `attention` marks a tile the operator has to act on. Tiles that read
     * zero must stay `neutral`: a healthy system shows them constantly, and a
     * tile that shouts while nothing is wrong is a tile that gets ignored.
     */
    tone?: 'neutral' | 'attention';
  } = $props();
</script>

<svelte:element
  this={href ? 'a' : 'div'}
  class="metric"
  class:attention={tone === 'attention'}
  {href}
  {title}
  role={href ? undefined : 'group'}
>
  <span class="label">{label}</span>
  <span class="value numeric">
    {value}{#if total !== undefined}<small>/{total}</small>{/if}
  </span>
  {#if detail}
    <span class="detail">
      {#if typeof detail === 'string'}{detail}{:else}{@render detail()}{/if}
    </span>
  {/if}
</svelte:element>

<style>
  .metric {
    display: grid;
    align-content: center;
    gap: 4px;
    padding: 14px 20px 14px 0;
  }

  a.metric:hover .value {
    color: var(--accent);
  }

  .label {
    color: var(--text-secondary);
    font-size: var(--text-compact);
    font-weight: 500;
  }

  .value {
    font-size: 24px;
    line-height: 30px;
    font-weight: 550;
    letter-spacing: -0.035em;
    transition: color 100ms ease;
  }

  .value small {
    margin-left: 2px;
    color: var(--text-tertiary);
    font-size: 14px;
    font-weight: 450;
  }

  .detail {
    color: var(--text-tertiary);
    font-size: var(--text-meta);
    line-height: var(--text-meta-line);
  }

  /* Attention is carried by the words first; colour only reinforces them. */
  .metric.attention .value {
    color: var(--danger);
  }

  .metric.attention .detail {
    color: var(--danger);
  }

  .metric.attention .detail :global(strong) {
    font-weight: 560;
  }
</style>
