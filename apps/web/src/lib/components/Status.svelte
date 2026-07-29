<script lang="ts">
  let {
    value,
    label
  }: {
    value: string;
    label?: string;
  } = $props();

  const tone = $derived(
    ['online', 'ready', 'completed_reported', 'healthy', 'accepted_by_spooler'].includes(value)
      ? 'success'
      : ['blocked', 'degraded', 'waiting_for_agent', 'needs_test', 'stale', 'dependency_missing'].includes(value)
        ? 'warning'
        : ['failed_terminal', 'failed_retryable', 'offline', 'failing', 'invalid', 'driver_mismatch', 'destination_missing', 'retired'].includes(value)
          ? 'danger'
          : ['delivery_uncertain'].includes(value)
            ? 'danger'
            : ['printing', 'spooling', 'agent_accepted', 'queued_local'].includes(value)
              ? 'info'
              : 'neutral'
  );

  const display = $derived(
    label ??
      value
        .replaceAll('_', ' ')
        .replace(/\b\w/g, (character) => character.toUpperCase())
  );
</script>

<span class="status {tone}">
  <span class="dot"></span>
  {display}
</span>

<style>
  .status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 20px;
    color: var(--text-secondary);
    font-size: 12px;
    white-space: nowrap;
  }

  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-tertiary);
    box-shadow: 0 0 0 2px color-mix(in oklch, var(--text-tertiary), transparent 86%);
  }

  .success {
    color: var(--success);
  }

  .success .dot {
    background: var(--success);
    box-shadow: 0 0 0 2px var(--success-soft);
  }

  .warning {
    color: var(--warning);
  }

  .warning .dot {
    background: var(--warning);
    box-shadow: 0 0 0 2px var(--warning-soft);
  }

  .danger {
    color: var(--danger);
  }

  .danger .dot {
    background: var(--danger);
    box-shadow: 0 0 0 2px var(--danger-soft);
  }

  .info {
    color: var(--info);
  }

  .info .dot {
    background: var(--info);
    box-shadow: 0 0 0 2px var(--info-soft);
  }
</style>
