<script lang="ts">
  import type { DashboardLoadError } from '$lib/server/dashboard-data';
  import Icon from './Icon.svelte';
  let { error }: { error: DashboardLoadError } = $props();
</script>

<section class="data-error" role="alert">
  <span><Icon name="warning" size={16} /></span>
  <div>
    <strong>{error.title}</strong>
    <p>{error.message}</p>
    <small class="mono">
      {error.code}{#if error.requestId} · {error.requestId}{/if}
    </small>
  </div>
  {#if error.retryable}<button class="button small" onclick={() => location.reload()}>Retry</button>{/if}
</section>

<style>
  .data-error {
    min-height: 72px;
    display: grid;
    grid-template-columns: 32px 1fr auto;
    align-items: center;
    gap: 11px;
    margin: 14px 0;
    padding: 11px 13px;
    background: var(--danger-soft);
    border: 1px solid color-mix(in oklch, var(--danger), transparent 72%);
    border-radius: var(--radius-lg);
  }
  .data-error > span {
    width: 31px;
    height: 31px;
    display: grid;
    place-items: center;
    color: var(--danger);
    background: color-mix(in oklch, var(--danger), transparent 84%);
    border-radius: 8px;
  }
  .data-error div { min-width: 0; display: grid; }
  strong { font-size: 11px; font-weight: 550; }
  p { margin: 2px 0 0; color: var(--text-secondary); font-size: 10px; line-height: 15px; }
  small { margin-top: 3px; color: var(--text-tertiary); font-size: 8px; }
  @media (max-width: 560px) {
    .data-error { grid-template-columns: 32px 1fr; }
    .data-error button { grid-column: 2; justify-self: start; }
  }
</style>
