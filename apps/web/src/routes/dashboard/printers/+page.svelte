<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  let { data } = $props();
  const agents = $derived(data.agents);
  const printers = $derived(data.printers);

  let query = $state('');
  let filterState = $state('all');
  const visible = $derived(
    printers.filter((printer) => {
      const matchesQuery =
        query === '' ||
        printer.name.toLowerCase().includes(query.toLowerCase()) ||
        printer.location?.toLowerCase().includes(query.toLowerCase());
      return matchesQuery && (filterState === 'all' || printer.state === filterState);
    })
  );
</script>

<svelte:head><title>Printers · Spool</title></svelte:head>

{#snippet actions()}
  <button class="button"><Icon name="activity" size={13} /> Refresh capabilities</button>
{/snippet}

<PageHeader
  title="Printers"
  description="Installed operating-system queues and driver-reported capabilities."
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

<div class="toolbar">
  <label class="search">
    <Icon name="search" size={13} />
    <input bind:value={query} aria-label="Search printers" placeholder="Search printers…" />
  </label>
  <select bind:value={filterState} aria-label="Filter printer state">
    <option value="all">All states</option>
    <option value="online">Online</option>
    <option value="degraded">Degraded</option>
    <option value="offline">Offline</option>
    <option value="paused">Paused</option>
  </select>
  <span class="count numeric">{visible.length} printers</span>
</div>

<div class="panel table-panel">
  <table>
    <thead>
      <tr>
        <th>Printer</th>
        <th>Status</th>
        <th>Agent</th>
        <th>Capabilities</th>
        <th>Queue</th>
        <th class="right">Last seen</th>
        <th><span class="sr-only">Actions</span></th>
      </tr>
    </thead>
    <tbody>
      {#each visible as printer}
        <tr>
          <td>
            <a class="printer" href={`/dashboard/printers/${printer.id}`}>
              <span class="printer-icon"><Icon name="printers" size={14} /></span>
              <span>
                <strong>{printer.name}</strong>
                <small>{printer.location ?? printer.description ?? 'No location'}</small>
              </span>
            </a>
          </td>
          <td>
            <Status value={printer.state} />
            {#if printer.stateReasons.length}
              <small class="reason">{printer.stateReasons[0]?.replaceAll('_', ' ')}</small>
            {/if}
          </td>
          <td>
            <span class="agent">
              <Icon name="agents" size={12} />
              {agents.find((agent) => agent.id === printer.agentId)?.name ?? 'Unknown'}
            </span>
          </td>
          <td>
            <div class="capabilities">
              <span>{printer.capabilities.color ? 'Color' : 'Mono'}</span>
              {#if printer.capabilities.duplex}<span>Duplex</span>{/if}
              <span>{printer.capabilities.dpis[0]}</span>
            </div>
          </td>
          <td class="numeric">{printer.queueDepth}</td>
          <td class="right muted numeric"><RelativeTime value={printer.lastSeenAt} /></td>
          <td class="action"><button aria-label={`Actions for ${printer.name}`}><Icon name="more" size={14} /></button></td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>

<style>
  .toolbar {
    min-height: 53px;
    display: flex;
    align-items: center;
    gap: 8px;
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

  select {
    height: 29px;
    padding: 0 25px 0 8px;
    color: var(--text-secondary);
    background: var(--surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    font-size: 10px;
  }

  .count {
    margin-left: auto;
    color: var(--text-tertiary);
    font-size: 10px;
  }

  .table-panel {
    overflow-x: auto;
  }

  table {
    width: 100%;
    min-width: 900px;
    border-collapse: collapse;
    font-size: 11px;
  }

  th {
    height: 31px;
    padding: 0 12px;
    color: var(--text-tertiary);
    font-size: 9px;
    font-weight: 500;
    text-align: left;
    text-transform: uppercase;
    letter-spacing: 0.035em;
    border-bottom: 1px solid var(--border-subtle);
  }

  td {
    height: 54px;
    padding: 0 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  tbody tr:last-child td {
    border-bottom: 0;
  }

  tbody tr:hover {
    background: color-mix(in oklch, var(--surface-hover), transparent 35%);
  }

  .printer {
    min-width: 240px;
    display: flex;
    align-items: center;
    gap: 9px;
  }

  .printer-icon {
    width: 29px;
    height: 29px;
    display: grid;
    flex: 0 0 auto;
    place-items: center;
    color: var(--text-secondary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: 7px;
  }

  .printer > span:last-child {
    display: grid;
    line-height: 15px;
  }

  .printer strong {
    font-weight: 500;
  }

  .printer small,
  .reason {
    color: var(--text-tertiary);
    font-size: 9px;
    text-transform: capitalize;
  }

  .reason {
    display: block;
    margin-left: 12px;
  }

  .agent {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-secondary);
  }

  .agent :global(svg) {
    color: var(--text-tertiary);
  }

  .capabilities {
    display: flex;
    gap: 4px;
  }

  .capabilities span {
    padding: 2px 5px;
    color: var(--text-tertiary);
    background: var(--surface-raised);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    font-size: 8px;
  }

  .right {
    text-align: right;
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
    cursor: pointer;
  }

  .action button:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

  @media (max-width: 620px) {
    .toolbar {
      flex-wrap: wrap;
      padding: 10px 0;
    }

    .search {
      width: 100%;
    }

    .count {
      display: none;
    }
  }
</style>
