<script lang="ts">
  let {
    label,
    value,
    total,
    detail,
    href
  }: {
    label: string;
    value: string | number;
    total?: string | number;
    detail?: string;
    href?: string;
  } = $props();
</script>

<svelte:element this={href ? 'a' : 'div'} class="metric" {href} role={href ? undefined : 'group'}>
  <span class="label">{label}</span>
  <span class="value numeric">
    {value}{#if total !== undefined}<small>/{total}</small>{/if}
  </span>
  {#if detail}<span class="detail">{detail}</span>{/if}
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
</style>
