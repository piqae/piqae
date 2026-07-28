<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import RelativeTime from '$lib/components/RelativeTime.svelte';
  import Status from '$lib/components/Status.svelte';
  let { data } = $props();
</script>

<svelte:head><title>{data.agent.name} · Spool</title></svelte:head>

{#snippet actions()}
  <button class="button">Diagnostics</button>
  <button class="button">Check for update</button>
  <button class="button ghost" aria-label="More actions"><Icon name="more" size={14} /></button>
{/snippet}

<PageHeader eyebrow="Agent" title={data.agent.name} description={data.agent.id} {actions} />
<div class="status-line"><Status value={data.agent.state} /><span>Seen <RelativeTime value={data.agent.lastSeenAt} /></span></div>

<div class="grid">
  <section class="panel">
    <header><h2>Installed printers</h2><span>{data.printers.length}</span></header>
    {#each data.printers as printer}
      <a class="printer" href={`/dashboard/printers/${printer.id}`}>
        <span class="icon"><Icon name="printers" size={14} /></span>
        <span><strong>{printer.name}</strong><small>{printer.location}</small></span>
        <Status value={printer.state} />
        <Icon name="arrow-right" size={12} />
      </a>
    {/each}
  </section>
  <aside class="panel">
    <header><h2>Runtime</h2></header>
    <dl>
      <div><dt>Platform</dt><dd>{data.agent.os} · {data.agent.architecture}</dd></div>
      <div><dt>Version</dt><dd class="mono">{data.agent.version}</dd></div>
      <div><dt>Protocol</dt><dd class="mono">{data.agent.protocolVersion}</dd></div>
      <div><dt>Local queue</dt><dd>{data.agent.queueDepth} jobs</dd></div>
    </dl>
  </aside>
</div>

<style>
  .status-line {
    height: 48px;
    display: flex;
    align-items: center;
    gap: 9px;
    color: var(--text-tertiary);
    font-size: 10px;
  }
  .grid { display: grid; grid-template-columns: 1fr 300px; gap: 12px; }
  section > header, aside > header { height: 43px; display: flex; align-items: center; justify-content: space-between; padding: 0 13px; border-bottom: 1px solid var(--border-subtle); }
  h2 { margin: 0; font-size: 11px; font-weight: 550; }
  header span { color: var(--text-tertiary); font-size: 9px; }
  .printer { min-height: 53px; display: grid; grid-template-columns: 30px 1fr auto 13px; align-items: center; gap: 9px; padding: 7px 12px; border-bottom: 1px solid var(--border-subtle); }
  .printer:last-child { border-bottom: 0; }
  .printer:hover { background: var(--surface-hover); }
  .icon { width: 29px; height: 29px; display: grid; place-items: center; color: var(--text-secondary); background: var(--surface-raised); border-radius: 7px; }
  .printer > span:nth-child(2) { display: grid; }
  .printer strong { font-size: 10px; font-weight: 500; }
  .printer small { color: var(--text-tertiary); font-size: 9px; }
  .printer > :global(svg) { color: var(--text-tertiary); }
  dl { margin: 0; padding: 7px 13px; }
  dl div { height: 31px; display: flex; align-items: center; justify-content: space-between; border-bottom: 1px solid var(--border-subtle); }
  dl div:last-child { border-bottom: 0; }
  dt { color: var(--text-tertiary); font-size: 9px; }
  dd { margin: 0; color: var(--text-secondary); font-size: 9px; text-transform: capitalize; }
  @media (max-width: 800px) { .grid { grid-template-columns: 1fr; } }
</style>
