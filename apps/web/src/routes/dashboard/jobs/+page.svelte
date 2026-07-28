<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  let { data } = $props();
  const jobs = $derived(data.jobs);
  const printers = $derived(data.printers);
  const agents = $derived(data.agents);

  let query = $state('');
  let filterState = $state('all');

  const visibleJobs = $derived(
    jobs.filter((job) => {
      const matchesQuery =
        query === '' ||
        job.title.toLowerCase().includes(query.toLowerCase()) ||
        job.id.toLowerCase().includes(query.toLowerCase());
      const matchesState =
        filterState === 'all' ||
        (filterState === 'active' &&
          !['completed_reported', 'cancelled', 'expired', 'failed_terminal'].includes(job.state)) ||
        (filterState === 'failed' && ['failed_terminal', 'failed_retryable'].includes(job.state)) ||
        job.state === filterState;
      return matchesQuery && matchesState;
    })
  );
</script>

<svelte:head><title>Jobs · Spool</title></svelte:head>

{#snippet actions()}
  <button class="button primary"><Icon name="plus" size={13} /> Print job</button>
{/snippet}

<PageHeader
  title="Jobs"
  description="Durable cloud, agent, and operating-system queue state."
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

<div class="toolbar">
  <label class="search">
    <span class="sr-only">Search jobs</span>
    <Icon name="search" size={13} />
    <input bind:value={query} placeholder="Search jobs…" />
  </label>
  <div class="filters" aria-label="Filter jobs by state">
    {#each ['all', 'active', 'failed', 'delivery_uncertain'] as option}
      <button class:active={filterState === option} onclick={() => (filterState = option)}>
        {option === 'delivery_uncertain' ? 'Uncertain' : option}
      </button>
    {/each}
  </div>
  <span class="result-count numeric">{visibleJobs.length} jobs</span>
</div>

<div class="panel table-panel">
  <table>
    <thead>
      <tr>
        <th>Job</th>
        <th>Status</th>
        <th>Printer</th>
        <th>Agent</th>
        <th>Authority</th>
        <th class="right">Updated</th>
        <th><span class="sr-only">Actions</span></th>
      </tr>
    </thead>
    <tbody>
      {#each visibleJobs as job}
        <tr>
          <td>
            <a class="job" href={`/dashboard/jobs/${job.id}`}>
              <strong>{job.title}</strong>
              <small class="mono">{job.id} · {job.contentFormat.toUpperCase()}</small>
            </a>
          </td>
          <td>
            <Status value={job.state} />
            {#if job.reasonCode}<small class="reason">{job.reasonCode.replaceAll('_', ' ')}</small>{/if}
          </td>
          <td>
            <span class="resource">
              <Icon name="printers" size={12} />
              {printers.find((printer) => printer.id === job.printerId)?.name ?? 'Unknown'}
            </span>
          </td>
          <td class="muted">
            {agents.find((agent) => agent.id === job.agentId)?.name ?? 'Unknown'}
          </td>
          <td class="muted">{job.authority.replaceAll('_', ' ')}</td>
          <td class="right muted numeric"><RelativeTime value={job.updatedAt} /></td>
          <td class="action"><button aria-label={`Actions for ${job.title}`}><Icon name="more" size={14} /></button></td>
        </tr>
      {:else}
        <tr>
          <td colspan="7">
            <div class="empty-state">No jobs match this view.</div>
          </td>
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
    gap: 10px;
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

  .search input {
    min-width: 0;
    width: 100%;
    color: var(--text-primary);
    background: transparent;
    border: 0;
    outline: 0;
    font-size: 11px;
  }

  .search input::placeholder {
    color: var(--text-tertiary);
  }

  .filters {
    display: flex;
    align-items: center;
    padding: 2px;
    background: var(--surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
  }

  .filters button {
    height: 23px;
    padding: 0 8px;
    color: var(--text-tertiary);
    text-transform: capitalize;
    background: transparent;
    border: 0;
    border-radius: var(--radius-sm);
    font-size: 10px;
    cursor: pointer;
  }

  .filters button:hover {
    color: var(--text-secondary);
  }

  .filters button.active {
    color: var(--text-primary);
    background: var(--surface-raised);
    box-shadow: inset 0 0 0 1px var(--border-default);
  }

  .result-count {
    margin-left: auto;
    color: var(--text-tertiary);
    font-size: 10px;
  }

  .table-panel {
    overflow-x: auto;
  }

  table {
    width: 100%;
    min-width: 950px;
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
    height: 51px;
    padding: 0 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  tbody tr:last-child td {
    border-bottom: 0;
  }

  tbody tr:hover {
    background: color-mix(in oklch, var(--surface-hover), transparent 35%);
  }

  .job {
    min-width: 220px;
    display: grid;
    line-height: 16px;
  }

  .job strong {
    font-weight: 500;
  }

  .job small {
    color: var(--text-tertiary);
    font-size: 9px;
  }

  .reason {
    display: block;
    margin-left: 12px;
    color: var(--text-tertiary);
    font-size: 9px;
  }

  .resource {
    max-width: 190px;
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .resource :global(svg) {
    flex: 0 0 auto;
    color: var(--text-tertiary);
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

  @media (max-width: 720px) {
    .toolbar {
      flex-wrap: wrap;
      padding: 11px 0;
    }

    .search {
      width: 100%;
    }

    .result-count {
      display: none;
    }
  }
</style>
