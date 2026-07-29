<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';

  let { data } = $props();
  const meta = $derived(data.meta);
</script>

<svelte:head><title>Developers · Piqae</title></svelte:head>

{#snippet actions()}
  <a class="button primary" href="/docs/quickstart"><Icon name="bolt" size={13} /> Quickstart</a>
{/snippet}

<PageHeader
  title="Developers"
  description="Credentials, event delivery, API references, and integration tools."
  {actions}
/>

<section class="cards" aria-label="Developer tools">
  <a class="panel card" href="/dashboard/api-keys">
    <span class="icon"><Icon name="api" size={16} /></span>
    <div><h2>API keys</h2><p>Create scoped credentials for test and live integrations.</p></div>
    <Icon name="arrow-right" size={13} />
  </a>
  <a class="panel card" href="/dashboard/webhooks">
    <span class="icon"><Icon name="webhooks" size={16} /></span>
    <div><h2>Webhooks</h2><p>Receive signed job, printer, and node events with retries.</p></div>
    <Icon name="arrow-right" size={13} />
  </a>
  <a class="panel card" href="/docs">
    <span class="icon"><Icon name="docs" size={16} /></span>
    <div><h2>Documentation</h2><p>Read the API quickstart, lifecycle model, and SDK guides.</p></div>
    <Icon name="arrow-right" size={13} />
  </a>
</section>

<section class="panel environment" aria-labelledby="environment-title">
  <header>
    <div><h2 id="environment-title">Environment</h2><p>Reported by this control plane through <code>GET /v1/meta</code>.</p></div>
    <span class="version mono">v{meta.version}</span>
  </header>
  <dl>
    <div><dt>Deployment</dt><dd>{meta.deployment.replace('_', ' ')}</dd></div>
    <div><dt>Authentication</dt><dd>{meta.auth.provider.replace('_', ' ')}</dd></div>
    <div><dt>Workspace switching</dt><dd>{meta.auth.workspaceSwitching ? 'Available' : 'Single workspace'}</dd></div>
    <div><dt>Updates</dt><dd>{meta.updates.officialFeed ? 'Official feed' : meta.updates.customFeed ? 'Custom feed' : 'Manual'}</dd></div>
  </dl>
</section>

<style>
  .cards { display: grid; grid-template-columns: repeat(3, minmax(220px, 1fr)); gap: 10px; padding-top: 18px; }
  .card { min-height: 104px; display: grid; grid-template-columns: 34px minmax(0, 1fr) 14px; align-items: center; gap: 12px; padding: 14px; }
  .card:hover { background: var(--surface-hover); border-color: var(--border-default); }
  .icon { width: 34px; height: 34px; display: grid; place-items: center; color: var(--text-secondary); background: var(--surface-raised); border: 1px solid var(--border-subtle); border-radius: 8px; }
  h2 { margin: 0; font-size: 11px; font-weight: 560; }
  p { margin: 3px 0 0; color: var(--text-tertiary); font-size: 9px; line-height: 14px; }
  .card > :global(svg) { color: var(--text-tertiary); }
  .environment { max-width: 780px; margin-top: 12px; }
  .environment header { min-height: 58px; display: flex; align-items: center; justify-content: space-between; gap: 20px; padding: 12px 14px; border-bottom: 1px solid var(--border-subtle); }
  .version { color: var(--text-tertiary); font-size: 9px; }
  code { color: var(--text-secondary); font-family: var(--font-mono); }
  dl { display: grid; grid-template-columns: repeat(2, 1fr); margin: 0; padding: 7px 14px; }
  dl div { min-height: 36px; display: flex; align-items: center; justify-content: space-between; gap: 20px; border-bottom: 1px solid var(--border-subtle); }
  dl div:nth-last-child(-n + 2) { border-bottom: 0; }
  dl div:nth-child(odd) { padding-right: 18px; border-right: 1px solid var(--border-subtle); }
  dl div:nth-child(even) { padding-left: 18px; }
  dt { color: var(--text-tertiary); font-size: 9px; }
  dd { margin: 0; color: var(--text-secondary); font-size: 9px; text-transform: capitalize; }
  @media (max-width: 780px) { .cards { grid-template-columns: 1fr; } }
  @media (max-width: 560px) { dl { grid-template-columns: 1fr; } dl div:nth-child(n) { padding: 0; border-right: 0; border-bottom: 1px solid var(--border-subtle); } dl div:last-child { border-bottom: 0; } }
</style>
