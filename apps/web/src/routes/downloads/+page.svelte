<script lang="ts">
  import Icon from '$lib/components/Icon.svelte';
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();

  function statusLabel(status: string) {
    if (status === 'supported') return 'Supported';
    if (status === 'preview') return 'Preview';
    if (status === 'development') return 'Development only';
    return 'Unavailable';
  }

  function formatDate(value: string) {
    return new Intl.DateTimeFormat(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric'
    }).format(new Date(value));
  }
</script>

<Seo
  title="Download the Spool native agent"
  description="See current Windows, macOS, and Linux Spool agent availability, signing state, checksums, minimum versions, and release notes."
  path="/downloads"
/>

<MarketingShell announcement="Downloads follow the checked-in release gates; preview builds are not presented as production">
<div class="downloads-page">
  <section class="intro" aria-labelledby="downloads-title">
    <div>
      <span class="eyebrow">Native nodes · {data.manifest.channel}</span>
      <h1 id="downloads-title">Connect a printer computer</h1>
      <p>
        Install Spool where the printer’s real driver already works. The node keeps its durable
        queue locally, captures native driver profiles, and connects to cloud or self-hosted Spool.
      </p>
    </div>
    {#if data.recommendedArtifactId}
      <div class="detected panel">
        <span><Icon name="check" size={14} /></span>
        <div>
          <strong>Detected {data.detected.label}</strong>
          <small>
            We highlighted the closest build
            {#if data.detected.architecture}· {data.detected.architecture}{/if}
          </small>
        </div>
      </div>
    {/if}
  </section>

  <section class="downloads" aria-label="Node downloads">
    {#each data.manifest.artifacts as artifact}
      <article
        class:recommended={artifact.id === data.recommendedArtifactId}
        class="release-card panel"
      >
        <header>
          <span class="platform"><Icon name={artifact.platform === 'linux' ? 'api' : 'agents'} size={18} /></span>
          <div>
            <div class="title-row">
              <h2>{artifact.title}</h2>
              {#if artifact.id === data.recommendedArtifactId}<span class="recommendation">Recommended</span>{/if}
            </div>
            <p>{artifact.statusReason}</p>
          </div>
        </header>

        <dl>
          <div><dt>Status</dt><dd><span class:verified={artifact.status === 'supported'} class:warning={artifact.status !== 'supported'} class="status">{statusLabel(artifact.status)}</span></dd></div>
          <div><dt>Version</dt><dd class="mono">v{artifact.version}</dd></div>
          <div><dt>Architecture</dt><dd>{artifact.architectures.join(' · ')}</dd></div>
          <div><dt>Minimum OS</dt><dd>{artifact.minimumOs}</dd></div>
          <div><dt>Signing</dt><dd>{artifact.signing.label}</dd></div>
          <div class="checksum">
            <dt>SHA-256</dt>
            <dd>
              {#if artifact.sha256}
                <code>{artifact.sha256}</code>
                {#if artifact.checksumUrl}
                  <a href={artifact.checksumUrl} target="_blank" rel="noreferrer">Sidecar</a>
                {/if}
              {:else}
                Not published
              {/if}
            </dd>
          </div>
        </dl>

        <ul>
          {#each artifact.notes as note}<li>{note}</li>{/each}
        </ul>

        <div class="card-actions">
          {#if artifact.downloadUrl && artifact.fileName}
            <a
              class="button primary"
              href={artifact.downloadUrl}
              data-marketing-download
              data-platform={artifact.platform}
            >
              Download {artifact.fileName} <Icon name="arrow-right" size={12} />
            </a>
          {:else}
            <a class="button" href={artifact.releaseUrl} target="_blank" rel="noreferrer">
              View build status <Icon name="external" size={12} />
            </a>
          {/if}
        </div>
      </article>
    {/each}
  </section>

  <section class="pairing panel" aria-labelledby="pairing-title">
    <div class="pairing-copy">
      <span class="eyebrow">Add node</span>
      <h2 id="pairing-title">Approve the computer in your browser</h2>
      <p>
        The native app creates its private device key locally. Browser pairing authorises that
        public identity without copying a permanent credential into the page.
      </p>
    </div>
    <ol>
      <li><span>1</span><div><strong>Install</strong><small>Run Spool as the user who owns the printer driver.</small></div></li>
      <li><span>2</span><div><strong>Connect node</strong><small>Choose Connect node from the tray or menu app.</small></div></li>
      <li><span>3</span><div><strong>Match and approve</strong><small>Confirm the computer details and one-time code in the browser.</small></div></li>
    </ol>
    <div class="pairing-actions">
      <a class="button primary" href="/pair">Open pairing <Icon name="arrow-right" size={12} /></a>
      <a class="button" href="/dashboard/nodes">View nodes</a>
    </div>
  </section>

  <section class="release-history" aria-labelledby="history-title">
    <div class="section-heading">
      <div>
        <span class="eyebrow">Release history</span>
        <h2 id="history-title">Older releases</h2>
      </div>
      <a class="button" href={data.manifest.releasesUrl} target="_blank" rel="noreferrer">
        All releases <Icon name="external" size={12} />
      </a>
    </div>

    {#if data.manifest.olderReleases.length > 0}
      <div class="history-list panel">
        {#each data.manifest.olderReleases as release}
          <a href={release.releaseUrl} target="_blank" rel="noreferrer">
            <div><strong>Spool v{release.version}</strong><span>{release.notes.join(' · ')}</span></div>
            <div><span class="status warning">{statusLabel(release.status)}</span><time datetime={release.publishedAt}>{formatDate(release.publishedAt)}</time></div>
          </a>
        {/each}
      </div>
    {:else}
      <div class="history-empty panel">
        <strong>No older server-owned releases are listed yet.</strong>
        <span>CI artifacts are not presented as supported downloads until their release evidence is published.</span>
      </div>
    {/if}
  </section>

  <section class="safety panel" aria-label="Release safety">
    <Icon name="warning" size={16} />
    <div>
      <strong>Read the status before installing</strong>
      <p>
        A checksum detects changed bytes only when it comes from a trusted channel. It does not
        replace platform signing, notarisation, clean-install testing, or physical printer
        certification.
      </p>
    </div>
    <a href={data.manifest.repositoryUrl} target="_blank" rel="noreferrer">
      Build from source <Icon name="external" size={11} />
    </a>
  </section>

  <footer>
    <span>
      {data.meta.deployment.replace('_', ' ')} · manifest v{data.manifest.currentVersion}
      {#if data.manifest.updatedAt} · updated {formatDate(data.manifest.updatedAt)}{/if}
    </span>
    <a href="/docs/quickstart">Installation guide</a>
  </footer>
</div>
</MarketingShell>

<style>
  .downloads-page {
    width: min(1120px, calc(100% - 32px));
    margin: 56px auto;
    padding: 54px 20px 30px;
    border: 1px solid rgb(255 255 255 / .08);
    border-radius: 22px;
    background: var(--m-dark);
    box-shadow: 0 30px 80px rgb(24 20 36 / .2);
  }
  .intro {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(250px, 320px);
    align-items: end;
    gap: 30px;
  }
  .intro > div:first-child { max-width: 670px; }
  .eyebrow {
    color: var(--accent);
    font-size: 10px;
    font-weight: 650;
    text-transform: uppercase;
    letter-spacing: .08em;
  }
  h1 { margin: 10px 0; font-size: clamp(30px, 5vw, 44px); line-height: 1.03; font-weight: 610; letter-spacing: -.045em; }
  .intro p, .pairing-copy p, .safety p {
    margin: 0;
    color: var(--text-secondary);
    font-size: 12px;
    line-height: 19px;
  }
  .detected {
    min-height: 66px;
    display: grid;
    grid-template-columns: 30px 1fr;
    align-items: center;
    gap: 10px;
    padding: 12px;
  }
  .detected > span {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    color: var(--success);
    background: var(--success-soft);
    border-radius: 7px;
  }
  .detected div { display: grid; gap: 3px; }
  .detected strong { font-size: 10px; font-weight: 560; }
  .detected small { color: var(--text-tertiary); font-size: 9px; }
  .downloads { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; margin-top: 32px; }
  .release-card { min-width: 0; display: flex; flex-direction: column; overflow: hidden; }
  .release-card.recommended { border-color: color-mix(in oklch, var(--accent), transparent 38%); box-shadow: inset 0 1px 0 var(--accent-soft); }
  .release-card > header { min-height: 112px; display: grid; grid-template-columns: 38px 1fr; gap: 12px; padding: 15px; border-bottom: 1px solid var(--border-subtle); }
  .platform { width: 36px; height: 36px; display: grid; place-items: center; color: var(--text-secondary); background: var(--surface-raised); border: 1px solid var(--border-default); border-radius: 9px; }
  .title-row { min-height: 22px; display: flex; align-items: center; justify-content: space-between; gap: 8px; }
  h2 { margin: 0; font-size: 13px; font-weight: 570; letter-spacing: -.015em; }
  .release-card header p { margin: 5px 0 0; color: var(--text-tertiary); font-size: 9px; line-height: 14px; }
  .recommendation { padding: 3px 6px; color: var(--accent); background: var(--accent-soft); border-radius: 999px; font-size: 8px; font-weight: 600; }
  dl { margin: 0; padding: 8px 14px; }
  dl > div { min-height: 29px; display: grid; grid-template-columns: 82px minmax(0, 1fr); align-items: center; gap: 9px; border-bottom: 1px solid var(--border-subtle); }
  dl > div:last-child { border-bottom: 0; }
  dt { color: var(--text-tertiary); font-size: 9px; }
  dd { min-width: 0; margin: 0; color: var(--text-secondary); font-size: 9px; text-align: right; }
  .checksum { align-items: start; padding: 8px 0; }
  .checksum dd { display: grid; justify-items: end; gap: 4px; }
  .checksum code { max-width: 100%; overflow-wrap: anywhere; color: var(--text-tertiary); font: 8px/12px var(--font-mono); }
  .checksum a { color: var(--text-secondary); text-decoration: underline; }
  .status { display: inline-flex; padding: 3px 6px; border-radius: 999px; font-size: 8px; font-weight: 600; }
  .status.verified { color: var(--success); background: var(--success-soft); }
  .status.warning { color: var(--warning); background: var(--warning-soft); }
  .release-card ul { flex: 1; margin: 0; padding: 5px 26px 13px; color: var(--text-tertiary); font-size: 9px; line-height: 15px; }
  .card-actions { padding: 0 14px 14px; }
  .card-actions .button { width: 100%; }
  .pairing { display: grid; grid-template-columns: 1.15fr 1.5fr auto; align-items: center; gap: 24px; margin-top: 12px; padding: 18px; }
  .pairing-copy h2 { margin: 6px 0; font-size: 17px; }
  .pairing ol { display: grid; gap: 8px; margin: 0; padding: 0; list-style: none; }
  .pairing li { display: grid; grid-template-columns: 24px 1fr; align-items: center; gap: 8px; }
  .pairing li > span { width: 22px; height: 22px; display: grid; place-items: center; color: var(--text-tertiary); background: var(--surface-raised); border: 1px solid var(--border-subtle); border-radius: 50%; font-size: 8px; }
  .pairing li div { display: grid; gap: 1px; }
  .pairing li strong { font-size: 9px; font-weight: 560; }
  .pairing li small { color: var(--text-tertiary); font-size: 8px; line-height: 12px; }
  .pairing-actions { display: grid; gap: 7px; }
  .release-history { margin-top: 34px; }
  .section-heading { display: flex; align-items: end; justify-content: space-between; margin-bottom: 10px; }
  .section-heading h2 { margin-top: 5px; }
  .history-list { overflow: hidden; }
  .history-list > a { min-height: 55px; display: flex; align-items: center; justify-content: space-between; gap: 18px; padding: 10px 13px; border-bottom: 1px solid var(--border-subtle); }
  .history-list > a:last-child { border-bottom: 0; }
  .history-list > a:hover { background: var(--surface-hover); }
  .history-list a > div { display: grid; gap: 3px; }
  .history-list a > div:last-child { justify-items: end; }
  .history-list strong { font-size: 10px; }
  .history-list span, .history-list time { color: var(--text-tertiary); font-size: 8px; }
  .history-empty { display: grid; gap: 4px; padding: 15px; }
  .history-empty strong { font-size: 10px; }
  .history-empty span { color: var(--text-tertiary); font-size: 9px; }
  .safety { display: grid; grid-template-columns: 24px 1fr auto; align-items: center; gap: 12px; margin-top: 12px; padding: 13px 15px; color: var(--warning); }
  .safety strong { font-size: 10px; }
  .safety p { margin-top: 3px; color: var(--text-tertiary); font-size: 9px; line-height: 14px; }
  .safety a { display: flex; align-items: center; gap: 5px; color: var(--text-secondary); font-size: 9px; white-space: nowrap; }
  footer { display: flex; justify-content: space-between; padding-top: 24px; color: var(--text-tertiary); font-size: 9px; }
  footer a:hover, .safety a:hover { color: var(--text-primary); }
  @media (max-width: 900px) {
    .intro { grid-template-columns: 1fr; align-items: start; }
    .detected { width: min(100%, 360px); }
    .downloads { grid-template-columns: 1fr 1fr; }
    .pairing { grid-template-columns: 1fr 1.3fr; }
    .pairing-actions { grid-column: 1 / -1; grid-template-columns: 1fr 1fr; }
  }
  @media (max-width: 660px) {
    .downloads-page { padding-top: 40px; }
    .downloads { grid-template-columns: 1fr; }
    .pairing { grid-template-columns: 1fr; }
    .pairing-actions { grid-column: auto; grid-template-columns: 1fr; }
    .safety { grid-template-columns: 24px 1fr; }
    .safety a { grid-column: 2; }
  }
  @media (max-width: 480px) {
    .section-heading { align-items: start; gap: 12px; }
    footer { display: grid; gap: 8px; }
  }
</style>
