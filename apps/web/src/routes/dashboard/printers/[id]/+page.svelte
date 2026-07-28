<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  let { data } = $props();
</script>

<svelte:head><title>{data.printer?.name ?? 'Printer unavailable'} · Spool</title></svelte:head>

{#if data.dataError}
  <PageHeader eyebrow="Printer" title="Printer unavailable" description={data.dataError.code} />
  <DataError error={data.dataError} />
{:else if data.printer}
{#snippet actions()}
  <button class="button"><Icon name="jobs" size={13} /> Test print</button>
  <button class="button">Pause queue</button>
{/snippet}

<PageHeader eyebrow="Printer" title={data.printer.name} description={data.printer.description ?? data.printer.id} {actions} />
<div class="status-line">
  <Status value={data.printer.state} />
  <span>{data.printer.location}</span><span>·</span><span>Seen <RelativeTime value={data.printer.lastSeenAt} /></span>
</div>

<div class="grid">
  <section class="panel jobs">
    <header><h2>Recent jobs</h2><span>{data.jobs.length}</span></header>
    {#each data.jobs as job}
      <a href={`/dashboard/jobs/${job.id}`}>
        <span><strong>{job.title}</strong><small class="mono">{job.id}</small></span>
        <Status value={job.state} />
        <RelativeTime value={job.updatedAt} />
        <Icon name="arrow-right" size={12} />
      </a>
    {:else}
      <div class="empty-state">No recent jobs for this printer.</div>
    {/each}
  </section>

  <aside>
    <section class="panel properties">
      <header><h2>Capabilities</h2></header>
      <dl>
        <div><dt>Color</dt><dd>{data.printer.capabilities.color ? 'Yes' : 'No'}</dd></div>
        <div><dt>Duplex</dt><dd>{data.printer.capabilities.duplex ? 'Yes' : 'No'}</dd></div>
        <div><dt>Copies</dt><dd>Up to {data.printer.capabilities.copies}</dd></div>
        <div><dt>Resolution</dt><dd>{data.printer.capabilities.dpis.join(', ')}</dd></div>
        <div><dt>Paper</dt><dd>{data.printer.capabilities.papers.join(', ')}</dd></div>
      </dl>
    </section>
    <section class="panel properties">
      <header><h2>Connection</h2></header>
      <dl>
        <div><dt>Agent</dt><dd>{data.agent?.name}</dd></div>
        <div><dt>Source</dt><dd>{data.printer.capabilities.source}</dd></div>
        <div><dt>Revision</dt><dd class="mono">{data.printer.capabilities.revision}</dd></div>
        <div><dt>Queue depth</dt><dd>{data.printer.queueDepth}</dd></div>
      </dl>
    </section>
  </aside>
</div>

<style>
  .status-line { height: 48px; display: flex; align-items: center; gap: 8px; color: var(--text-tertiary); font-size: 10px; }
  .grid { display: grid; grid-template-columns: 1fr 320px; align-items: start; gap: 12px; }
  .panel > header { height: 43px; display: flex; align-items: center; justify-content: space-between; padding: 0 13px; border-bottom: 1px solid var(--border-subtle); }
  h2 { margin: 0; font-size: 11px; font-weight: 550; }
  header span { color: var(--text-tertiary); font-size: 9px; }
  .jobs > a { min-height: 50px; display: grid; grid-template-columns: 1fr auto 60px 13px; align-items: center; gap: 12px; padding: 7px 12px; border-bottom: 1px solid var(--border-subtle); }
  .jobs > a:last-child { border-bottom: 0; }
  .jobs > a:hover { background: var(--surface-hover); }
  .jobs a > span:first-child { display: grid; line-height: 15px; }
  .jobs strong { font-size: 10px; font-weight: 500; }
  .jobs small { color: var(--text-tertiary); font-size: 8px; }
  .jobs a > :global(time) { color: var(--text-tertiary); font-size: 9px; text-align: right; }
  .jobs a > :global(svg) { color: var(--text-tertiary); }
  aside { display: grid; gap: 12px; }
  dl { margin: 0; padding: 7px 13px; }
  dl div { min-height: 31px; display: grid; grid-template-columns: 80px 1fr; align-items: center; border-bottom: 1px solid var(--border-subtle); }
  dl div:last-child { border-bottom: 0; }
  dt { color: var(--text-tertiary); font-size: 9px; }
  dd { margin: 0; overflow: hidden; color: var(--text-secondary); font-size: 9px; text-align: right; text-overflow: ellipsis; white-space: nowrap; }
  @media (max-width: 850px) { .grid { grid-template-columns: 1fr; } }
</style>
{/if}
