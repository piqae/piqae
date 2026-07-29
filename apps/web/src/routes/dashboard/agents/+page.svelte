<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  let { data } = $props();
  const agents = $derived(data.agents);

  let query = $state('');
  const visible = $derived(
    agents.filter(
      (agent) =>
        query === '' ||
        agent.name.toLowerCase().includes(query.toLowerCase()) ||
        agent.labels.some((label) => label.includes(query.toLowerCase()))
    )
  );
</script>

<svelte:head><title>Agents · Spool</title></svelte:head>

{#snippet actions()}
  <a class="button" href="/docs/quickstart"><Icon name="docs" size={13} /> Install guide</a>
  <button class="button primary" disabled title="Agent enrolment UI is not implemented"><Icon name="plus" size={13} /> Enrol agent</button>
{/snippet}

<PageHeader
  title="Agents"
  description="Installed services that discover printers and own local durable queues."
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

<div class="toolbar">
  <label class="search">
    <Icon name="search" size={13} />
    <input bind:value={query} aria-label="Search agents" placeholder="Search agents…" />
  </label>
  <span class="count numeric">{visible.length} agents</span>
</div>

<section class="agent-grid">
  {#each visible as agent}
    <article class="panel">
      <header>
        <span class="os-icon"><Icon name="agents" size={15} /></span>
        <div class="title">
          <strong>{agent.name}</strong>
          <span class="mono">{agent.id}</span>
        </div>
        <a class="agent-details" aria-label={`View ${agent.name}`} href={`/dashboard/agents/${agent.id}`}><Icon name="arrow-right" size={13} /></a>
      </header>
      <div class="health">
        <Status value={agent.state} />
        <span>Seen <RelativeTime value={agent.lastSeenAt} /></span>
      </div>
      <dl>
        <div><dt>Platform</dt><dd>{agent.os} · {agent.architecture}</dd></div>
        <div><dt>Agent version</dt><dd class="mono">v{agent.version}</dd></div>
        <div><dt>Printers</dt><dd class="numeric">{agent.printerCount}</dd></div>
        <div><dt>Local queue</dt><dd class="numeric">{agent.queueDepth} jobs</dd></div>
      </dl>
      <footer>
        <div class="labels">
          {#each agent.labels as label}<span>{label}</span>{/each}
        </div>
        <a href={`/dashboard/agents/${agent.id}`}>Details <Icon name="arrow-right" size={11} /></a>
      </footer>
    </article>
  {/each}
</section>

<style>
  .toolbar {
    min-height: 53px;
    display: flex;
    align-items: center;
  }

  .search {
    width: 220px;
    height: 29px;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0 8px;
    color: var(--text-tertiary);
    background: var(--surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
  }

  .search:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 2px var(--accent-soft);
  }

  input {
    min-width: 0;
    width: 100%;
    color: var(--text-primary);
    background: transparent;
    border: 0;
    outline: 0;
    font-size: 11px;
  }

  .count {
    margin-left: auto;
    color: var(--text-tertiary);
    font-size: 10px;
  }

  .agent-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(260px, 1fr));
    gap: 10px;
  }

  article {
    overflow: hidden;
  }

  article > header {
    height: 54px;
    display: grid;
    grid-template-columns: 31px 1fr 26px;
    align-items: center;
    gap: 9px;
    padding: 0 11px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .os-icon {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: 8px;
  }

  .title {
    min-width: 0;
    display: grid;
    line-height: 15px;
  }

  .title strong {
    overflow: hidden;
    font-size: 11px;
    font-weight: 540;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .title span {
    color: var(--text-tertiary);
    font-size: 8px;
  }

  .agent-details {
    width: 25px;
    height: 25px;
    display: grid;
    place-items: center;
    color: var(--text-tertiary);
    border-radius: var(--radius-sm);
  }

  .agent-details:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  .health {
    height: 35px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .health > span {
    color: var(--text-tertiary);
    font-size: 9px;
  }

  dl {
    margin: 0;
    padding: 7px 12px;
  }

  dl div {
    height: 27px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }

  dt {
    color: var(--text-tertiary);
    font-size: 10px;
  }

  dd {
    margin: 0;
    color: var(--text-secondary);
    font-size: 10px;
    text-transform: capitalize;
  }

  footer {
    min-height: 39px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 6px 11px;
    background: color-mix(in oklch, var(--canvas), transparent 35%);
    border-top: 1px solid var(--border-subtle);
  }

  .labels {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }

  .labels span {
    padding: 2px 5px;
    color: var(--text-tertiary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    font-size: 8px;
  }

  footer a {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--text-secondary);
    font-size: 9px;
    white-space: nowrap;
  }

  footer a:hover {
    color: var(--text-primary);
  }

  @media (max-width: 1100px) {
    .agent-grid {
      grid-template-columns: repeat(2, minmax(260px, 1fr));
    }
  }

  @media (max-width: 620px) {
    .agent-grid {
      grid-template-columns: 1fr;
    }

    .search {
      width: 100%;
    }

    .count {
      display: none;
    }
  }
</style>
