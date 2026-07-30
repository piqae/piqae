<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  import {
    DataPanel,
    SearchField,
    SegmentedControl,
    Toolbar
  } from '$lib/components/ui';
  let { data } = $props();
  const jobs = $derived(data.jobs);
  const printers = $derived(data.printers);
  const agents = $derived(data.agents);

  let query = $state('');
  let filterState = $state('all');
  const jobFilters = [
    { value: 'all', label: 'All' },
    { value: 'active', label: 'Active' },
    { value: 'failed', label: 'Failed' },
    { value: 'delivery_uncertain', label: 'Uncertain' }
  ];

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

<svelte:head><title>Jobs · Piqae</title></svelte:head>

{#snippet actions()}
  <a class="button primary" href="/docs/quickstart"><Icon name="plus" size={13} /> Print via API</a>
{/snippet}

<PageHeader
  title="Jobs"
  description="Durable cloud, node, and operating-system queue state."
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

<Toolbar meta={`${visibleJobs.length} jobs`}>
  <SearchField bind:value={query} label="Search jobs" placeholder="Search jobs…" />
  <SegmentedControl bind:value={filterState} label="Filter jobs by state" options={jobFilters} />
</Toolbar>

<DataPanel minWidth="950px">
  <table class="ui-data-table">
    <thead>
      <tr>
        <th>Job</th>
        <th>Status</th>
        <th>Printer</th>
        <th>Node</th>
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
          <td class="action"><a class="row-details" aria-label={`View ${job.title}`} href={`/dashboard/jobs/${job.id}`}><Icon name="arrow-right" size={13} /></a></td>
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
</DataPanel>

<style>
  .ui-data-table td {
    height: 51px;
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
