<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  let { data } = $props();
  const apiKeys = $derived(data.apiKeys);
  let copied = $state<string | null>(null);

  async function copyPrefix(prefix: string) {
    await navigator.clipboard.writeText(prefix);
    copied = prefix;
    setTimeout(() => (copied = null), 1200);
  }
</script>

<svelte:head><title>API keys · Spool</title></svelte:head>

{#snippet actions()}
  <button class="button primary"><Icon name="plus" size={13} /> Create secret key</button>
{/snippet}

<PageHeader
  title="API keys"
  description="Scoped credentials for applications that submit and inspect print jobs."
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

<section class="security-note">
  <Icon name="api" size={15} />
  <div>
    <strong>Secret values are shown once</strong>
    <p>Spool stores a one-way hash. Rotate a key immediately if its value is exposed.</p>
  </div>
  <a href="/docs/api-keys">Security guide <Icon name="arrow-right" size={11} /></a>
</section>

<div class="panel">
  <table>
    <thead>
      <tr><th>Name</th><th>Key</th><th>Environment</th><th>Scopes</th><th>Last used</th><th>Created</th><th></th></tr>
    </thead>
    <tbody>
      {#each apiKeys as key}
        <tr>
          <td><strong>{key.name}</strong></td>
          <td>
            <button class="copy" onclick={() => copyPrefix(key.prefix)} title="Copy key prefix">
              <code>{key.prefix}••••••••••</code>
              <Icon name={copied === key.prefix ? 'check' : 'copy'} size={11} />
            </button>
          </td>
          <td><span class:live={key.environment === 'live'} class="environment">{key.environment}</span></td>
          <td><span class="scope-count">{key.scopes.length} scopes</span></td>
          <td class="muted">{#if key.lastUsedAt}<RelativeTime value={key.lastUsedAt} />{:else}Never{/if}</td>
          <td class="muted"><RelativeTime value={key.createdAt} /></td>
          <td class="action"><button aria-label={`Actions for ${key.name}`}><Icon name="more" size={14} /></button></td>
        </tr>
      {/each}
      {#if apiKeys.length === 0 && !data.dataError}
        <tr><td colspan="7"><div class="empty-state">No API keys created.</div></td></tr>
      {/if}
    </tbody>
  </table>
</div>

<section class="quickstart panel">
  <div>
    <span class="step">Next step</span>
    <h2>Send your first print job</h2>
    <p>Use the test environment until your integration is ready for physical printers.</p>
  </div>
  <pre><code><span>curl</span> https://api.spool.dev/v1/jobs \
  -H <em>"Authorization: Bearer $SPOOL_API_KEY"</em> \
  -H <em>"Idempotency-Key: order-481"</em> \
  -H <em>"Content-Type: application/json"</em> \
  -d @job.json</code></pre>
  <a class="button" href="/docs/quickstart">Open quick start <Icon name="arrow-right" size={12} /></a>
</section>

<style>
  .security-note {
    min-height: 57px;
    display: grid;
    grid-template-columns: 30px 1fr auto;
    align-items: center;
    gap: 10px;
    margin: 13px 0;
    padding: 9px 12px;
    color: var(--info);
    background: var(--info-soft);
    border: 1px solid color-mix(in oklch, var(--info), transparent 75%);
    border-radius: var(--radius-md);
  }

  .security-note strong {
    color: var(--text-primary);
    font-size: 10px;
    font-weight: 540;
  }

  .security-note p {
    margin: 2px 0 0;
    color: var(--text-secondary);
    font-size: 9px;
  }

  .security-note a {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: 9px;
  }

  .panel {
    overflow-x: auto;
  }

  table {
    width: 100%;
    min-width: 780px;
    border-collapse: collapse;
    font-size: 10px;
  }

  th {
    height: 31px;
    padding: 0 12px;
    color: var(--text-tertiary);
    font-size: 8px;
    font-weight: 500;
    text-align: left;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-bottom: 1px solid var(--border-subtle);
  }

  td {
    height: 49px;
    padding: 0 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  tr:last-child td {
    border-bottom: 0;
  }

  td strong {
    font-weight: 500;
  }

  .copy {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 6px;
    color: var(--text-secondary);
    background: var(--canvas);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .copy code {
    font: 9px var(--font-mono);
  }

  .copy :global(svg) {
    color: var(--text-tertiary);
  }

  .environment,
  .scope-count {
    padding: 2px 6px;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    font-size: 8px;
    text-transform: capitalize;
  }

  .environment.live {
    color: var(--success);
    background: var(--success-soft);
  }

  .action {
    width: 38px;
    padding: 0 6px;
  }

  .action button {
    width: 25px;
    height: 25px;
    display: grid;
    place-items: center;
    color: var(--text-tertiary);
    background: transparent;
    border: 0;
    border-radius: var(--radius-sm);
  }

  .quickstart {
    display: grid;
    grid-template-columns: minmax(210px, 0.55fr) minmax(360px, 1fr) auto;
    align-items: center;
    gap: 20px;
    margin-top: 12px;
    padding: 16px;
  }

  .step {
    color: var(--accent);
    font-size: 8px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  h2 {
    margin: 3px 0 0;
    font-size: 12px;
    font-weight: 550;
  }

  .quickstart p {
    margin: 3px 0 0;
    color: var(--text-tertiary);
    font-size: 9px;
    line-height: 14px;
  }

  pre {
    overflow-x: auto;
    margin: 0;
    padding: 9px 11px;
    color: var(--text-secondary);
    background: var(--canvas);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    font: 9px/15px var(--font-mono);
  }

  pre span {
    color: var(--accent-hover);
  }

  pre em {
    color: var(--success);
    font-style: normal;
  }

  @media (max-width: 960px) {
    .quickstart {
      grid-template-columns: 1fr;
    }

    .quickstart .button {
      justify-self: start;
    }
  }

  @media (max-width: 620px) {
    .security-note {
      grid-template-columns: 30px 1fr;
    }

    .security-note a {
      display: none;
    }
  }
</style>
