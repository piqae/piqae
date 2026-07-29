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
  const setupStep = $derived(
    overview.agents.total === 0 ? 1 : overview.printers.total === 0 ? 2 : overview.jobs.recent === 0 ? 3 : 4
  );
</script>

<svelte:head>
  <title>Overview · Spool</title>
  <meta name="description" content="Current print nodes, printers, jobs, and actionable conditions." />
</svelte:head>

{#snippet actions()}
  <a class="button" href="/docs/quickstart"><Icon name="docs" size={13} /> API quickstart</a>
  <a class="button primary" href="/dashboard/nodes"><Icon name="plus" size={13} /> Add node</a>
{/snippet}

<PageHeader
  title="Overview"
  description="Current operational state across your nodes, printers, and durable queues."
  {actions}
/>

{#if data.dataError}<DataError error={data.dataError} />{/if}

<section class="metrics" aria-label="Printing overview">
  <a class="metric" href="/dashboard/nodes">
    <div class="metric-label">Nodes online</div>
    <div class="metric-value numeric">{overview.agents.online}<span>/{overview.agents.total}</span></div>
    <div class="metric-detail">
      {overview.agents.total - overview.agents.online} unavailable
    </div>
  </a>
  <a class="metric" href="/dashboard/printers">
    <div class="metric-label">Printers available</div>
    <div class="metric-value numeric">{overview.printers.online}<span>/{overview.printers.total}</span></div>
    <div class="metric-detail">{overview.printers.attention} need attention</div>
  </a>
  <a class="metric" href="/dashboard/jobs">
    <div class="metric-label">Recent jobs</div>
    <div class="metric-value numeric">{overview.jobs.recent.toLocaleString()}</div>
    <div class="metric-detail">{overview.jobs.active} active in queues</div>
  </a>
  <a class="metric" href="/dashboard/jobs?state=failed">
    <div class="metric-label">Needs review</div>
    <div class="metric-value numeric">{overview.jobs.failed + overview.jobs.uncertain}</div>
    <div class="metric-detail">{overview.jobs.uncertain} uncertain handoff</div>
  </a>
</section>

<section class="overview-grid">
  <div class="panel onboarding">
    <header class="section-header">
      <div>
        <h2>{setupStep === 4 ? 'First print complete' : 'Send your first print'}</h2>
        <p>One native node, one discovered printer, then one API request.</p>
      </div>
      <span class="progress">{Math.min(setupStep - 1, 3)}/3</span>
    </header>
    <ol>
      <li class:complete={setupStep > 1} class:current={setupStep === 1}>
        <span>{setupStep > 1 ? '✓' : '1'}</span>
        <div><strong>Add a node</strong><small>Install Spool on the computer connected to your printer.</small></div>
        <a href="/dashboard/nodes">Open nodes <Icon name="arrow-right" size={11} /></a>
      </li>
      <li class:complete={setupStep > 2} class:current={setupStep === 2}>
        <span>{setupStep > 2 ? '✓' : '2'}</span>
        <div><strong>Confirm a printer</strong><small>Spool discovers installed queues and their native profiles.</small></div>
        <a href="/dashboard/printers">View printers <Icon name="arrow-right" size={11} /></a>
      </li>
      <li class:complete={setupStep > 3} class:current={setupStep === 3}>
        <span>{setupStep > 3 ? '✓' : '3'}</span>
        <div><strong>Submit a PDF</strong><small>Create an API key and send a durable print job.</small></div>
        <a href="/docs/quickstart">Quickstart <Icon name="arrow-right" size={11} /></a>
      </li>
    </ol>
  </div>

  <div class="panel attention-panel">
    <header class="section-header">
      <div><h2>Needs attention</h2><p>Printer conditions reported now.</p></div>
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
      {:else}
        <div class="empty-attention">No printer conditions require attention.</div>
      {/each}
    </div>
  </div>
</section>

<section class="panel jobs">
  <header class="section-header">
    <div><h2>Recent jobs</h2><p>Newest recorded state across all printers.</p></div>
    <a class="button small ghost" href="/dashboard/jobs">View all <Icon name="arrow-right" size={12} /></a>
  </header>
  <div class="table-wrap">
    <table>
      <thead><tr><th>Job</th><th>Status</th><th>Printer</th><th>Source</th><th class="right">Updated</th></tr></thead>
      <tbody>
        {#each recentJobs as job}
          <tr>
            <td><a class="job-link" href={`/dashboard/jobs/${job.id}`}><strong>{job.title}</strong><span class="mono">{job.id}</span></a></td>
            <td><Status value={job.state} /></td>
            <td>{printers.find((printer) => printer.id === job.printerId)?.name ?? 'Unknown'}</td>
            <td class="muted">{job.source ?? '—'}</td>
            <td class="right muted numeric"><RelativeTime value={job.updatedAt} /></td>
          </tr>
        {:else}
          <tr><td colspan="5"><div class="empty-state compact">No recent jobs.</div></td></tr>
        {/each}
      </tbody>
    </table>
  </div>
</section>

<style>
  .metrics { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); border-bottom: 1px solid var(--border-subtle); }
  .metric { min-height: 108px; display: grid; align-content: center; padding: 16px 20px; border-right: 1px solid var(--border-subtle); }
  .metric:first-child { padding-left: 0; }
  .metric:last-child { border-right: 0; }
  .metric:hover { background: color-mix(in oklch, var(--surface-hover), transparent 45%); }
  .metric-label { margin-bottom: 5px; color: var(--text-secondary); font-size: 11px; font-weight: 500; }
  .metric-value { font-size: 25px; line-height: 30px; font-weight: 550; letter-spacing: -0.04em; }
  .metric-value span { margin-left: 2px; color: var(--text-tertiary); font-size: 14px; font-weight: 450; }
  .metric-detail { margin-top: 7px; color: var(--text-tertiary); font-size: 10px; }
  .overview-grid { display: grid; grid-template-columns: minmax(0, 1.45fr) minmax(280px, .75fr); gap: 12px; margin-top: 18px; }
  .section-header { min-height: 58px; display: flex; align-items: center; justify-content: space-between; gap: 16px; padding: 12px 14px; border-bottom: 1px solid var(--border-subtle); }
  h2 { margin: 0; font-size: 12px; line-height: 18px; font-weight: 560; }
  .section-header p { margin: 1px 0 0; color: var(--text-tertiary); font-size: 10px; line-height: 15px; }
  .progress, .count { min-width: 25px; height: 22px; display: grid; place-items: center; color: var(--text-secondary); background: var(--surface-raised); border: 1px solid var(--border-subtle); border-radius: 6px; font-size: 9px; }
  ol { margin: 0; padding: 5px 0; list-style: none; }
  li { min-height: 62px; display: grid; grid-template-columns: 25px minmax(0, 1fr) auto; align-items: center; gap: 10px; padding: 8px 13px; border-bottom: 1px solid var(--border-subtle); }
  li:last-child { border-bottom: 0; }
  li > span { width: 22px; height: 22px; display: grid; place-items: center; color: var(--text-tertiary); background: var(--surface-raised); border: 1px solid var(--border-default); border-radius: 50%; font-size: 9px; }
  li.complete > span { color: var(--success); background: var(--success-soft); border-color: transparent; }
  li.current > span { color: white; background: var(--accent); border-color: var(--accent); }
  li div { display: grid; gap: 2px; }
  li strong { font-size: 10px; font-weight: 530; }
  li small { color: var(--text-tertiary); font-size: 9px; }
  li a { display: flex; align-items: center; gap: 6px; color: var(--text-secondary); font-size: 9px; }
  li a:hover { color: var(--text-primary); }
  .attention-list a { min-height: 55px; display: grid; grid-template-columns: 28px 1fr 14px; align-items: center; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--border-subtle); }
  .attention-list a:hover { background: var(--surface-hover); }
  .attention-icon { width: 27px; height: 27px; display: grid; place-items: center; color: var(--warning); background: var(--warning-soft); border-radius: 7px; }
  .attention-copy { min-width: 0; display: grid; line-height: 16px; }
  .attention-copy strong { overflow: hidden; font-size: 11px; font-weight: 500; text-overflow: ellipsis; white-space: nowrap; }
  .attention-copy small { color: var(--text-tertiary); font-size: 10px; text-transform: capitalize; }
  .empty-attention { min-height: 170px; display: grid; place-items: center; padding: 20px; color: var(--text-tertiary); font-size: 10px; text-align: center; }
  .jobs { margin-top: 12px; }
  .table-wrap { overflow-x: auto; }
  table { width: 100%; border-collapse: collapse; font-size: 11px; }
  th { height: 30px; padding: 0 13px; color: var(--text-tertiary); font-size: 9px; font-weight: 500; text-align: left; text-transform: uppercase; letter-spacing: .035em; border-bottom: 1px solid var(--border-subtle); }
  td { height: 47px; padding: 0 13px; border-bottom: 1px solid var(--border-subtle); white-space: nowrap; }
  tr:last-child td { border-bottom: 0; }
  tbody tr:hover { background: color-mix(in oklch, var(--surface-hover), transparent 34%); }
  .right { text-align: right; }
  .job-link { min-width: 220px; display: grid; line-height: 15px; }
  .job-link strong { font-weight: 500; }
  .job-link span { color: var(--text-tertiary); font-size: 9px; }
  .empty-state.compact { min-height: 100px; }
  @media (max-width: 900px) { .metrics { grid-template-columns: repeat(2, 1fr); } .metric:nth-child(2) { border-right: 0; } .metric:nth-child(-n + 2) { border-bottom: 1px solid var(--border-subtle); } .overview-grid { grid-template-columns: 1fr; } }
  @media (max-width: 620px) { .metric { min-height: 92px; padding: 12px; } .metric:nth-child(odd) { padding-left: 0; } li { grid-template-columns: 25px 1fr; } li a { grid-column: 2; } }
</style>
