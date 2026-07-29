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
  <a class="button" href={`/dashboard/nodes/${data.printer.agentId}`}>
    <Icon name="agents" size={13} /> Open node
  </a>
{/snippet}

<PageHeader eyebrow="Destination" title={data.printer.name} description={data.printer.description ?? data.printer.id} {actions} />
<div class="status-line">
  <Status value={data.printer.state} />
  <span>{data.printer.location}</span><span>·</span><span>Seen <RelativeTime value={data.printer.lastSeenAt} /></span>
</div>

<section class="panel profiles">
  <header>
    <span>
      <h2>Native print profiles</h2>
      <small>Driver settings are captured and edited on {data.agent?.name ?? 'the node'}.</small>
    </span>
    <span class="profile-count numeric">{data.printer.profiles.length}</span>
  </header>
  {#each data.printer.profiles as profile}
    <article class="profile">
      <div class="profile-main">
        <span class="profile-icon"><Icon name="printers" size={13} /></span>
        <span>
          <strong>{profile.name}{profile.isDefault ? ' — Default' : ''}</strong>
          <small>
            Revision {profile.revision} · {profile.nativeKind.replaceAll('_', ' ')}
            {profile.published ? ' · Published' : ' · Local only'}
          </small>
        </span>
      </div>
      <Status value={profile.status} />
      <dl class="profile-summary">
        <div><dt>Paper / stock</dt><dd>{profile.summary.paper ?? profile.stockId ?? 'Driver default'}</dd></div>
        <div><dt>Source</dt><dd>{profile.summary.source ?? 'Driver default'}</dd></div>
        <div><dt>Output</dt><dd>{[profile.summary.color, profile.summary.resolution].filter(Boolean).join(' · ') || 'Native settings'}</dd></div>
      </dl>
      <div class="profile-meta">
        <span>Safe overrides: {profile.safeOverrides.join(', ') || 'none'}</span>
        {#if profile.lastValidatedAt}
          <span>Validated <RelativeTime value={profile.lastValidatedAt} /></span>
        {:else}
          <span>Not yet validated</span>
        {/if}
      </div>
      <div class="profile-actions">
        <button class="button compact" disabled title="Open the Spool tray application on this node to edit native driver settings">
          Edit on node
        </button>
        <button class="button compact" disabled title="Test profiles from the Spool tray application on this node">
          Test
        </button>
      </div>
    </article>
  {:else}
    <div class="empty-state profile-empty">
      No print profiles. Open the Spool tray application on this node and choose Add profile.
    </div>
  {/each}
</section>

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
        <div><dt>Node</dt><dd>{data.agent?.name}</dd></div>
        <div><dt>Source</dt><dd>{data.printer.capabilities.source}</dd></div>
        <div><dt>Revision</dt><dd class="mono">{data.printer.capabilities.revision}</dd></div>
        <div><dt>Queue depth</dt><dd>{data.printer.queueDepth}</dd></div>
      </dl>
    </section>
  </aside>
</div>

<style>
  .status-line { height: 48px; display: flex; align-items: center; gap: 8px; color: var(--text-tertiary); font-size: 10px; }
  .profiles { margin-bottom: 12px; }
  .profiles > header { min-height: 52px; display: flex; align-items: center; justify-content: space-between; padding: 8px 13px; border-bottom: 1px solid var(--border-subtle); }
  .profiles > header > span:first-child { display: grid; gap: 3px; }
  .profiles header small { color: var(--text-tertiary); font-size: 9px; }
  .profile-count { color: var(--text-tertiary); font-size: 10px; }
  .profile { min-height: 82px; display: grid; grid-template-columns: minmax(230px, 1fr) 150px minmax(260px, 1.2fr) minmax(180px, .8fr) auto; align-items: center; gap: 16px; padding: 10px 13px; border-bottom: 1px solid var(--border-subtle); }
  .profile:last-child { border-bottom: 0; }
  .profile-main { display: flex; align-items: center; gap: 9px; min-width: 0; }
  .profile-main > span:last-child { min-width: 0; display: grid; gap: 3px; }
  .profile-main strong { overflow: hidden; font-size: 10px; font-weight: 550; text-overflow: ellipsis; white-space: nowrap; }
  .profile-main small { color: var(--text-tertiary); font-size: 8px; text-transform: capitalize; }
  .profile-icon { width: 29px; height: 29px; display: grid; flex: 0 0 auto; place-items: center; color: var(--text-secondary); background: var(--surface-raised); border: 1px solid var(--border-subtle); border-radius: 7px; }
  .profile-summary { display: grid; gap: 3px; margin: 0; padding: 0; }
  .profile-summary div { min-height: 0; display: grid; grid-template-columns: 74px 1fr; border: 0; }
  .profile-summary dt, .profile-summary dd { font-size: 8px; }
  .profile-summary dd { text-align: left; }
  .profile-meta { display: grid; gap: 4px; color: var(--text-tertiary); font-size: 8px; }
  .profile-actions { display: flex; gap: 5px; }
  .compact { min-height: 25px; padding: 0 8px; font-size: 9px; }
  .profile-empty { min-height: 92px; }
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
  @media (max-width: 1100px) {
    .profile { grid-template-columns: 1fr auto; }
    .profile-summary, .profile-meta { grid-column: 1 / -1; }
    .profile-actions { grid-column: 2; grid-row: 2 / span 2; }
  }
  @media (max-width: 850px) { .grid { grid-template-columns: 1fr; } }
</style>
{/if}
