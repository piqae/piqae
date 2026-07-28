<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  let { data } = $props();

  const overview = $derived(data.overview);
  const recentJobs = $derived(data.jobs);
  const printers = $derived(data.printers);
  const attentionPrinters = $derived(printers.filter((printer) => printer.state !== 'online'));
  const bars = [38, 42, 47, 44, 51, 58, 53, 65, 71, 63, 76, 82, 74, 88, 90, 86, 94, 78, 85, 92, 87, 96, 91, 76];
</script>

<svelte:head>
  <title>Overview · Spool</title>
  <meta
    name="description"
    content="Real-time printing health, queue activity, and operational status."
  />
</svelte:head>

{#snippet actions()}
  <a class="button" href="/docs/quickstart"><Icon name="docs" size={13} /> Quick start</a>
  <button class="button primary"><Icon name="plus" size={13} /> Print job</button>
{/snippet}

<PageHeader
  title="Overview"
  description="Live operational state across your agents, printers, and queues."
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

<section class="metrics" aria-label="Printing overview">
  <a class="metric" href="/dashboard/agents">
    <div class="metric-label">Agents online</div>
    <div class="metric-value numeric">{overview.agents.online}<span>/{overview.agents.total}</span></div>
    <div class="metric-detail good"><span></span>All expected agents seen</div>
  </a>
  <a class="metric" href="/dashboard/printers">
    <div class="metric-label">Printers available</div>
    <div class="metric-value numeric">{overview.printers.online}<span>/{overview.printers.total}</span></div>
    <div class="metric-detail warn"><span></span>{overview.printers.attention} need attention</div>
  </a>
  <a class="metric" href="/dashboard/jobs">
    <div class="metric-label">Jobs today</div>
    <div class="metric-value numeric">{overview.jobs.today.toLocaleString()}</div>
    <div class="metric-detail"><span></span>{overview.jobs.active} active in queues</div>
  </a>
  <a class="metric" href="/dashboard/jobs?state=failed">
    <div class="metric-label">Needs review</div>
    <div class="metric-value numeric">{overview.jobs.failed + overview.jobs.uncertain}</div>
    <div class="metric-detail danger"><span></span>{overview.jobs.uncertain} uncertain handoff</div>
  </a>
</section>

<section class="grid">
  <div class="panel activity-panel">
    <header class="section-header">
      <div>
        <h2>Print activity</h2>
        <p>Successful OS queue handoffs over the last 24 hours</p>
      </div>
      <div class="latency">
        <span>Pickup p95</span>
        <strong class="numeric">{overview.pickupLatencyP95Ms} ms</strong>
      </div>
    </header>
    <div class="chart" aria-label="Hourly print activity chart">
      <div class="axis">
        <span>120</span><span>80</span><span>40</span><span>0</span>
      </div>
      <div class="bars">
        {#each bars as value, index}
          <span
            class:recent={index > 19}
            style={`height: ${value}%`}
            title={`${value} successful jobs`}
          ></span>
        {/each}
      </div>
    </div>
    <div class="chart-footer">
      <span>00:00</span><span>06:00</span><span>12:00</span><span>18:00</span><span>Now</span>
    </div>
  </div>

  <div class="panel attention-panel">
    <header class="section-header">
      <div>
        <h2>Needs attention</h2>
        <p>Actionable fleet conditions</p>
      </div>
      <span class="count">{attentionPrinters.length}</span>
    </header>
    <div class="attention-list">
      {#each attentionPrinters as printer}
        <a href={`/dashboard/printers/${printer.id}`}>
          <span class="attention-icon"><Icon name="warning" size={14} /></span>
          <span class="attention-copy">
            <strong>{printer.name}</strong>
            <small>{printer.stateReasons[0]?.replaceAll('_', ' ') ?? printer.state}</small>
          </span>
          <Icon name="arrow-right" size={13} />
        </a>
      {/each}
      {#if attentionPrinters.length === 0}
        <div class="empty-attention">No printer conditions require attention.</div>
      {/if}
    </div>
    <a class="attention-footer" href="/dashboard/printers">
      Review printer fleet <Icon name="arrow-right" size={13} />
    </a>
  </div>
</section>

<section class="panel jobs">
  <header class="section-header">
    <div>
      <h2>Recent jobs</h2>
      <p>Newest state transitions across all printers</p>
    </div>
    <a class="button small ghost" href="/dashboard/jobs">View all <Icon name="arrow-right" size={12} /></a>
  </header>
  <div class="table-wrap">
    <table>
      <thead>
        <tr>
          <th>Job</th>
          <th>Status</th>
          <th>Printer</th>
          <th>Source</th>
          <th class="right">Updated</th>
        </tr>
      </thead>
      <tbody>
        {#each recentJobs as job}
          <tr>
            <td>
              <a class="job-link" href={`/dashboard/jobs/${job.id}`}>
                <strong>{job.title}</strong>
                <span class="mono">{job.id}</span>
              </a>
            </td>
            <td><Status value={job.state} /></td>
            <td>{printers.find((printer) => printer.id === job.printerId)?.name ?? 'Unknown'}</td>
            <td class="muted">{job.source ?? '—'}</td>
            <td class="right muted numeric"><RelativeTime value={job.updatedAt} /></td>
          </tr>
        {:else}
          <tr><td colspan="5"><div class="empty-state">No recent jobs.</div></td></tr>
        {/each}
      </tbody>
    </table>
  </div>
</section>

<style>
  .metrics {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    border-bottom: 1px solid var(--border-subtle);
  }

  .metric {
    min-height: 113px;
    display: grid;
    align-content: center;
    padding: 16px 20px;
    border-right: 1px solid var(--border-subtle);
    transition: background-color 90ms ease;
  }

  .metric:first-child {
    padding-left: 0;
  }

  .metric:last-child {
    border-right: 0;
  }

  .metric:hover {
    background: color-mix(in oklch, var(--surface-hover), transparent 45%);
  }

  .metric-label {
    margin-bottom: 5px;
    color: var(--text-secondary);
    font-size: 11px;
    font-weight: 500;
  }

  .metric-value {
    font-size: 25px;
    line-height: 30px;
    font-weight: 550;
    letter-spacing: -0.04em;
  }

  .metric-value span {
    margin-left: 2px;
    color: var(--text-tertiary);
    font-size: 14px;
    font-weight: 450;
  }

  .metric-detail {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 7px;
    color: var(--text-tertiary);
    font-size: 10px;
  }

  .metric-detail span {
    width: 5px;
    height: 5px;
    background: var(--info);
    border-radius: 50%;
  }

  .metric-detail.good {
    color: var(--success);
  }

  .metric-detail.good span {
    background: var(--success);
  }

  .metric-detail.warn {
    color: var(--warning);
  }

  .metric-detail.warn span {
    background: var(--warning);
  }

  .metric-detail.danger {
    color: var(--danger);
  }

  .metric-detail.danger span {
    background: var(--danger);
  }

  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1.7fr) minmax(280px, 0.8fr);
    gap: 12px;
    margin-top: 18px;
  }

  .section-header {
    min-height: 58px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 12px 14px;
    border-bottom: 1px solid var(--border-subtle);
  }

  h2 {
    margin: 0;
    font-size: 12px;
    line-height: 18px;
    font-weight: 560;
  }

  .section-header p {
    margin: 1px 0 0;
    color: var(--text-tertiary);
    font-size: 10px;
    line-height: 15px;
  }

  .latency {
    display: grid;
    justify-items: end;
    color: var(--text-tertiary);
    font-size: 9px;
  }

  .latency strong {
    color: var(--text-secondary);
    font-size: 12px;
    font-weight: 500;
  }

  .chart {
    height: 190px;
    display: grid;
    grid-template-columns: 30px 1fr;
    padding: 18px 16px 0 12px;
  }

  .axis {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding-bottom: 1px;
    color: var(--text-tertiary);
    font-size: 9px;
    font-variant-numeric: tabular-nums;
  }

  .bars {
    display: flex;
    align-items: end;
    gap: clamp(2px, 0.5vw, 5px);
    background: repeating-linear-gradient(
      to bottom,
      transparent 0,
      transparent calc(33.333% - 1px),
      var(--border-subtle) calc(33.333% - 1px),
      var(--border-subtle) 33.333%
    );
    border-bottom: 1px solid var(--border-subtle);
  }

  .bars span {
    min-width: 2px;
    flex: 1;
    background: color-mix(in oklch, var(--accent), transparent 48%);
    border-radius: 2px 2px 0 0;
  }

  .bars span.recent {
    background: color-mix(in oklch, var(--accent), transparent 16%);
  }

  .chart-footer {
    display: flex;
    justify-content: space-between;
    padding: 7px 17px 12px 42px;
    color: var(--text-tertiary);
    font-size: 9px;
    font-variant-numeric: tabular-nums;
  }

  .count {
    min-width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    color: var(--warning);
    background: var(--warning-soft);
    border-radius: 6px;
    font-size: 10px;
  }

  .attention-list a {
    min-height: 55px;
    display: grid;
    grid-template-columns: 28px 1fr 14px;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border-subtle);
  }

  .empty-attention {
    min-height: 90px;
    display: grid;
    place-items: center;
    padding: 12px;
    color: var(--text-tertiary);
    font-size: 10px;
    text-align: center;
  }

  .attention-list a:hover {
    background: var(--surface-hover);
  }

  .attention-icon {
    width: 27px;
    height: 27px;
    display: grid;
    place-items: center;
    color: var(--warning);
    background: var(--warning-soft);
    border-radius: 7px;
  }

  .attention-copy {
    min-width: 0;
    display: grid;
    line-height: 16px;
  }

  .attention-copy strong {
    overflow: hidden;
    font-size: 11px;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .attention-copy small {
    color: var(--text-tertiary);
    font-size: 10px;
    text-transform: capitalize;
  }

  .attention-list a > :global(svg) {
    color: var(--text-tertiary);
  }

  .attention-footer {
    height: 38px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 13px;
    color: var(--text-secondary);
    font-size: 10px;
  }

  .attention-footer:hover {
    color: var(--text-primary);
  }

  .jobs {
    margin-top: 12px;
  }

  .table-wrap {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 11px;
  }

  th {
    height: 30px;
    padding: 0 13px;
    color: var(--text-tertiary);
    font-size: 9px;
    font-weight: 500;
    text-align: left;
    text-transform: uppercase;
    letter-spacing: 0.035em;
    border-bottom: 1px solid var(--border-subtle);
  }

  td {
    height: 47px;
    padding: 0 13px;
    border-bottom: 1px solid var(--border-subtle);
    white-space: nowrap;
  }

  tr:last-child td {
    border-bottom: 0;
  }

  tbody tr:hover {
    background: color-mix(in oklch, var(--surface-hover), transparent 34%);
  }

  .right {
    text-align: right;
  }

  .job-link {
    min-width: 220px;
    display: grid;
    line-height: 15px;
  }

  .job-link strong {
    font-weight: 500;
  }

  .job-link span {
    color: var(--text-tertiary);
    font-size: 9px;
  }

  @media (max-width: 1000px) {
    .metrics {
      grid-template-columns: repeat(2, 1fr);
    }

    .metric:nth-child(2) {
      border-right: 0;
    }

    .metric:nth-child(-n + 2) {
      border-bottom: 1px solid var(--border-subtle);
    }

    .metric:nth-child(3) {
      padding-left: 0;
    }

    .grid {
      grid-template-columns: 1fr;
    }
  }

  @media (max-width: 620px) {
    .metrics {
      grid-template-columns: 1fr 1fr;
    }

    .metric {
      min-height: 95px;
      padding: 12px;
    }

    .metric:nth-child(odd) {
      padding-left: 0;
    }

    .metric-value {
      font-size: 21px;
    }
  }
</style>
