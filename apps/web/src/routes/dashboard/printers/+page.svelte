<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  import { DataPanel, SearchField, Toolbar } from '$lib/components/ui';
  let { data } = $props();
  const agents = $derived(data.agents);
  const printers = $derived(data.printers);

  let query = $state('');
  let filterState = $state('all');
  const readyProfileCount = (printer: (typeof printers)[number]) =>
    printer.profiles.filter((profile) => profile.status === 'ready').length;
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

<svelte:head><title>Printers · Piqae</title></svelte:head>

{#snippet actions()}
  <button class="button" disabled title="Capability refresh mutation is not implemented"><Icon name="activity" size={13} /> Refresh capabilities</button>
{/snippet}

<PageHeader
  title="Printers"
  description="Installed destinations, native profiles, and operational readiness."
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

<Toolbar meta={`${visible.length} printers`}>
  <SearchField bind:value={query} label="Search printers" placeholder="Search printers…" />
  <select class="ui-select" bind:value={filterState} aria-label="Filter printer state">
    <option value="all">All states</option>
    <option value="online">Online</option>
    <option value="degraded">Degraded</option>
    <option value="offline">Offline</option>
    <option value="paused">Paused</option>
  </select>
</Toolbar>

<DataPanel minWidth="900px">
  <table class="ui-data-table">
    <thead>
      <tr>
        <th>Printer</th>
        <th>Status</th>
        <th>Node</th>
        <th>Profiles</th>
        <th>Readiness</th>
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
            <strong class="numeric">{printer.profiles.length}</strong>
            <small class="profile-copy">
              {printer.profiles.filter((profile) => profile.published).length} published
            </small>
          </td>
          <td>
            {#if printer.profiles.length === 0}
              <span class="muted">No profiles</span>
            {:else if readyProfileCount(printer) === printer.profiles.length}
              <Status value="online" label={`${readyProfileCount(printer)} ready`} />
            {:else}
              <Status value="degraded" label={`${printer.profiles.length - readyProfileCount(printer)} need attention`} />
            {/if}
          </td>
          <td class="numeric">{printer.queueDepth}</td>
          <td class="right muted numeric"><RelativeTime value={printer.lastSeenAt} /></td>
          <td class="action"><a class="row-details" aria-label={`View ${printer.name}`} href={`/dashboard/printers/${printer.id}`}><Icon name="arrow-right" size={13} /></a></td>
        </tr>
      {/each}
    </tbody>
  </table>
</DataPanel>

<style>
  .ui-data-table td {
    height: 54px;
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

  .profile-copy {
    display: block;
    margin-top: 2px;
    color: var(--text-tertiary);
    font-size: 8px;
  }

  .right {
    text-align: right;
  }

  .action {
    width: 38px;
    padding: 0 6px;
  }

  .row-details {
    width: 25px;
    height: 25px;
    display: grid;
    place-items: center;
    color: var(--text-tertiary);
    border-radius: var(--radius-sm);
  }

  .row-details:hover {
    color: var(--text-primary);
    background: var(--surface-hover);
  }

</style>
