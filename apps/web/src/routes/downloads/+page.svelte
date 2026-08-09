<script lang="ts">
  import { onMount } from 'svelte';
  import { replaceState } from '$app/navigation';
  import Icon from '$lib/components/Icon.svelte';
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import type { PageData } from './$types';
  import { consumeNodeConnectFragment, nativeNodeConnectUrl } from '$lib/node-connect-fragment';

  let { data }: { data: PageData } = $props();
  let invitation = $state<ReturnType<typeof consumeNodeConnectFragment>>(null);

  onMount(() => {
    invitation = consumeNodeConnectFragment(window.location, (url) => replaceState(url, {}));
  });

  function openPiqae() {
    if (!invitation) return;
    const url = nativeNodeConnectUrl(invitation.enrolmentToken, invitation.controlPlaneUrl);
    if (url) window.location.assign(url);
  }

  type Artifact = PageData['manifest']['artifacts'][number];

  const detectedArtifact = $derived(
    data.manifest.artifacts.find((artifact) => artifact.id === data.recommendedArtifactId) ?? null
  );

  function platformName(platform: Artifact['platform']) {
    if (platform === 'macos') return 'macOS';
    if (platform === 'windows') return 'Windows';
    return 'Linux';
  }

  function shortPlatformName(platform: Artifact['platform']) {
    return platform === 'macos' ? 'Mac' : platformName(platform);
  }

  function isDownloadable(artifact: Artifact) {
    return (
      (artifact.status === 'supported' || artifact.status === 'preview') &&
      Boolean(artifact.downloadUrl && artifact.fileName && artifact.sha256) &&
      (artifact.signing.status === 'verified' ||
        (artifact.status === 'preview' && artifact.signing.status === 'unsigned'))
    );
  }

  function isUnsignedPreview(artifact: Artifact) {
    return artifact.status === 'preview' && artifact.signing.status === 'unsigned';
  }

  function downloadLabel(artifact: Artifact) {
    if (isUnsignedPreview(artifact)) {
      return `Download unsigned prerelease for ${shortPlatformName(artifact.platform)}`;
    }
    if (artifact.signing.status === 'verified') {
      return `Download release for ${shortPlatformName(artifact.platform)}`;
    }
    return `Download for ${shortPlatformName(artifact.platform)}`;
  }

  function statusLabel(status: Artifact['status']) {
    if (status === 'supported') return 'Stable';
    if (status === 'preview') return 'Preview';
    if (status === 'development') return 'In development';
    return 'Unavailable';
  }

  function artifactStatusLabel(artifact: Artifact) {
    if (isUnsignedPreview(artifact)) return 'Unsigned prerelease';
    if (artifact.signing.status === 'verified') return 'Release';
    return statusLabel(artifact.status);
  }

  function supportLabel(artifact: Artifact) {
    return artifact.status === 'preview' ? 'Preview support' : statusLabel(artifact.status);
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
  title="Download Piqae"
  description="Download the Piqae native printing agent for macOS, Windows, or Linux."
  path="/downloads"
/>

<MarketingShell>
  {#if invitation}
    <section class="connect-session" aria-labelledby="connect-session-title">
      <div class="m-container connect-session-inner">
        <div>
          <span class="m-eyebrow">Printer computer setup</span>
          <h2 id="connect-session-title">Connect the Piqae app on this computer.</h2>
          <p>
            Install Piqae if needed, then open it to review the requesting service and choose
            which local printers it may use. You do not need a separate Piqae account.
          </p>
          <p class="connect-warning">
            Keep this page open while installing. The invitation expires shortly and works once.
          </p>
        </div>
        <button class="m-button dark" type="button" onclick={openPiqae}>Open Piqae to connect</button>
      </div>
    </section>
  {/if}
  <section class="download-hero">
    <div class="hero-inner m-container">
      <div class="hero-copy-panel">
        {#if detectedArtifact}
          <span class="m-eyebrow">Piqae for {platformName(detectedArtifact.platform)}</span>
          <h1>
            {#if isDownloadable(detectedArtifact)}
              {#if detectedArtifact.signing.status === 'verified'}
                Download Piqae for {platformName(detectedArtifact.platform)}
              {:else}
                Download Piqae for {platformName(detectedArtifact.platform)}
              {/if}
            {:else}
              Piqae for {platformName(detectedArtifact.platform)} is almost ready
            {/if}
          </h1>
          <p class="hero-copy">
            Connect this computer to the printers it already knows. Piqae works quietly in the
            background so your apps can print from anywhere.
          </p>

          {#if isUnsignedPreview(detectedArtifact)}
            <p class="unsigned-warning" role="alert">
              <strong>Unsigned prerelease.</strong> Windows will show an unknown-publisher or
              SmartScreen warning. Use this evaluation build only if you accept that risk; automatic
              updates are disabled.
            </p>
          {/if}

          <div class="hero-actions">
            {#if isDownloadable(detectedArtifact) && detectedArtifact.downloadUrl}
              <a
                class="m-button primary download-button"
                href={detectedArtifact.downloadUrl}
                data-marketing-download
                data-platform={detectedArtifact.platform}
              >
                {downloadLabel(detectedArtifact)}
                <Icon name="arrow-right" size={14} />
              </a>
            {:else}
              <a
                class="m-button primary download-button"
                href={detectedArtifact.releaseUrl}
                target="_blank"
                rel="noreferrer"
              >
                View {shortPlatformName(detectedArtifact.platform)} release status
                <Icon name="external" size={13} />
              </a>
            {/if}
            <a class="text-link" href="#other-downloads">Other platforms <span>↓</span></a>
          </div>

          <p class="release-line">
            {artifactStatusLabel(detectedArtifact)} v{detectedArtifact.version} ·
            {supportLabel(detectedArtifact)} ·
            {detectedArtifact.architectures.join(' + ')}
          </p>
        {:else}
          <span class="m-eyebrow">Native printing agent</span>
          <h1>Download Piqae</h1>
          <p class="hero-copy">
            Choose the computer connected to your printer. Piqae supports macOS, Windows, and Linux.
          </p>
          <a class="m-button primary download-button" href="#other-downloads">
            Choose your platform <span>↓</span>
          </a>
        {/if}
      </div>

      <div
        class="hero-visual"
        role="img"
        aria-label="Piqae connecting a computer to its installed printers"
      >
        <span class="visual-orbit orbit-one" aria-hidden="true"></span>
        <span class="visual-orbit orbit-two" aria-hidden="true"></span>
        <div class="agent-window">
          <div class="window-bar">
            <span></span><span></span><span></span>
            <strong>Piqae</strong>
            <small>Connected</small>
          </div>
          <div class="agent-main">
            <div class="agent-heading">
              <span><Icon name="agents" size={22} /></span>
              <div><small>This computer</small><strong>Ready to print</strong></div>
              <i></i>
            </div>
            <div class="printer-row">
              <span><Icon name="printers" size={18} /></span>
              <div><strong>Shipping labels</strong><small>Ready · Native driver</small></div>
              <b>Ready</b>
            </div>
            <div class="printer-row">
              <span><Icon name="printers" size={18} /></span>
              <div><strong>Production printer</strong><small>Ready · Native driver</small></div>
              <b>Ready</b>
            </div>
          </div>
        </div>
        <div class="print-sheet sheet-one" aria-hidden="true"><span></span><i></i><i></i></div>
        <div class="print-sheet sheet-two" aria-hidden="true"><span></span><i></i><i></i></div>
      </div>
    </div>
  </section>

  <section id="other-downloads" class="platforms m-container" aria-labelledby="platforms-title">
    <div class="section-heading">
      <span class="m-eyebrow">Other platforms</span>
      <h2 id="platforms-title">Piqae for every printer computer.</h2>
    </div>

    <div class="platform-list">
      {#each data.manifest.artifacts as artifact}
        <article class:detected={artifact.id === data.recommendedArtifactId}>
          <span class="platform-icon" aria-hidden="true">
            <Icon name={artifact.platform === 'linux' ? 'api' : 'agents'} size={23} strokeWidth={1.45} />
          </span>
          <div class="platform-copy">
            <div>
              <h3>{platformName(artifact.platform)}</h3>
              {#if artifact.id === data.recommendedArtifactId}<span class="you-are-here">This device</span>{/if}
            </div>
            <p>{artifact.minimumOs}</p>
            <small>{artifact.architectures.join(' · ')} · {artifactStatusLabel(artifact)} · {supportLabel(artifact)}</small>
            {#if isUnsignedPreview(artifact)}
              <p class="platform-warning">Unsigned evaluation build · Unknown publisher warning expected</p>
            {/if}
          </div>
          {#if isDownloadable(artifact) && artifact.downloadUrl}
            <a
              class="platform-action"
              href={artifact.downloadUrl}
              data-marketing-download
              data-platform={artifact.platform}
              aria-label={downloadLabel(artifact)}
            >
              {isUnsignedPreview(artifact) ? 'Download unsigned prerelease' : artifact.signing.status === 'verified' ? 'Download release' : 'Download'}
              <Icon name="arrow-right" size={12} />
            </a>
          {:else}
            <a
              class="platform-action"
              href={artifact.releaseUrl}
              target="_blank"
              rel="noreferrer"
              aria-label={`View ${platformName(artifact.platform)} release status`}
            >
              View status <Icon name="external" size={11} />
            </a>
          {/if}
        </article>
      {/each}
    </div>
  </section>

  <section class="setup">
    <div class="m-container">
      <div class="section-heading">
        <span class="m-eyebrow">Three simple steps</span>
        <h2>Ready to print in minutes.</h2>
      </div>

      <ol class="steps">
        <li>
          <span>1</span>
          <h3>Install Piqae</h3>
          <p>Run the installer on the Mac, PC, or Linux computer connected to your printer.</p>
        </li>
        <li>
          <span>2</span>
          <h3>Connect your account</h3>
          <p>Open Piqae and approve the computer securely in your browser.</p>
        </li>
        <li>
          <span>3</span>
          <h3>Choose your printers</h3>
          <p>Your installed drivers and their full print capabilities are ready to use.</p>
        </li>
      </ol>

      <div class="setup-actions">
        <a class="m-button dark" href="/pair">Connect a computer <Icon name="arrow-right" size={13} /></a>
        <a class="m-button" href="/docs/quickstart">Read the installation guide</a>
      </div>
    </div>
  </section>

  <section class="release-details m-narrow" aria-labelledby="details-title">
    <div class="section-heading">
      <span class="m-eyebrow">Release details</span>
      <h2 id="details-title">Everything technical, when you need it.</h2>
      <p>Versions, signing information, checksums, and release notes for each platform.</p>
    </div>

    <div class="details-list">
      {#each data.manifest.artifacts as artifact}
        <details>
          <summary>
            <span>
              <strong>{platformName(artifact.platform)}</strong>
              <small>v{artifact.version} · {artifactStatusLabel(artifact)} · {supportLabel(artifact)}</small>
            </span>
            <Icon name="chevron-down" size={16} />
          </summary>
          <div class="detail-content">
            <p class="status-reason">{artifact.statusReason}</p>
            <dl>
              <div><dt>Architecture</dt><dd>{artifact.architectures.join(' · ')}</dd></div>
              <div><dt>Minimum OS</dt><dd>{artifact.minimumOs}</dd></div>
              <div><dt>Signing</dt><dd>{artifact.signing.label}</dd></div>
              <div>
                <dt>SHA-256</dt>
                <dd>
                  {#if artifact.sha256}
                    <code>{artifact.sha256}</code>
                    {#if artifact.checksumUrl}
                      <a href={artifact.checksumUrl} target="_blank" rel="noreferrer">Checksum file</a>
                    {/if}
                  {:else}
                    Published with the signed release
                  {/if}
                </dd>
              </div>
            </dl>
            <ul>
              {#each artifact.notes as note}<li>{note}</li>{/each}
            </ul>
            <div class="detail-links">
              <a href={artifact.releaseUrl} target="_blank" rel="noreferrer">
                Release notes <Icon name="external" size={10} />
              </a>
              <a href="/docs/quickstart">Manual installation</a>
            </div>
          </div>
        </details>
      {/each}
    </div>

    <div class="history">
      <span>
        {data.manifest.channel} channel · manifest v{data.manifest.currentVersion}
        {#if data.manifest.updatedAt} · updated {formatDate(data.manifest.updatedAt)}{/if}
      </span>
      <a href={data.manifest.releasesUrl} target="_blank" rel="noreferrer">
        All releases <Icon name="external" size={10} />
      </a>
    </div>
  </section>
</MarketingShell>

<style>
  .connect-session {
    border-bottom: 1px solid var(--m-border);
    background: var(--m-surface-soft, #f6f5f1);
  }
  .connect-session-inner {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 32px;
    align-items: center;
    padding-block: 28px;
  }
  .connect-session h2 { margin: 7px 0 8px; font-size: clamp(24px, 3vw, 34px); }
  .connect-session p { max-width: 760px; margin: 0; color: var(--m-muted); line-height: 1.55; }
  .connect-session .connect-warning { margin-top: 8px; font-size: 13px; }
  @media (max-width: 720px) {
    .connect-session-inner { grid-template-columns: 1fr; }
    .connect-session button { width: 100%; }
  }
  .download-hero {
    position: relative;
    border-bottom: 1px solid var(--m-border);
    background: #fff;
  }
  .hero-inner {
    min-height: 720px;
    display: grid;
    grid-template-columns: minmax(0, .94fr) minmax(460px, 1.06fr);
    align-items: center;
    gap: clamp(54px, 8vw, 112px);
    padding-block: clamp(72px, 8vw, 112px);
  }
  .hero-copy-panel {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
  }
  .hero-copy-panel .m-eyebrow { margin-bottom: 18px; }
  h1 {
    max-width: 610px;
    margin: 0;
    font-family: var(--m-font-editorial);
    font-size: clamp(52px, 6.3vw, 82px);
    font-weight: 400;
    letter-spacing: -.055em;
    line-height: .94;
    text-wrap: balance;
  }
  .hero-copy {
    max-width: 540px;
    margin: 28px 0 0;
    color: var(--m-muted);
    font-size: clamp(16px, 1.7vw, 19px);
    line-height: 1.55;
    text-wrap: pretty;
  }
  .unsigned-warning {
    max-width: 560px;
    margin: 22px 0 0;
    padding: 14px 16px;
    border: 1px solid #d8a12d;
    border-radius: 10px;
    background: #fff8e6;
    color: #5d4100;
    font-size: 13px;
    line-height: 1.5;
  }
  .hero-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 16px 20px;
    margin-top: 34px;
  }
  .download-button {
    min-height: 58px !important;
    padding-inline: 26px !important;
    border-radius: 999px !important;
    font-size: 16px !important;
  }
  .text-link {
    color: var(--m-muted);
    font-size: 13px;
    font-weight: 650;
  }
  .text-link:hover { color: var(--m-ink); }
  .text-link span { margin-left: 4px; }
  .release-line {
    margin: 22px 0 0;
    color: var(--m-faint);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: .055em;
  }
  .hero-visual {
    position: relative;
    min-height: 560px;
    display: grid;
    place-items: center;
    overflow: hidden;
    border-radius: 22px;
    background:
      radial-gradient(circle at 76% 20%, rgb(255 255 255 / .6), transparent 26%),
      radial-gradient(circle at 15% 86%, rgb(0 106 255 / .24), transparent 32%),
      #bcd8ff;
    box-shadow: inset 0 0 0 1px rgb(0 66 160 / .08);
  }
  .visual-orbit {
    position: absolute;
    width: 470px;
    height: 470px;
    border: 1px solid rgb(0 81 190 / .13);
    border-radius: 50%;
  }
  .orbit-two { width: 350px; height: 350px; }
  .agent-window {
    position: relative;
    z-index: 2;
    width: min(78%, 440px);
    overflow: hidden;
    border: 1px solid rgb(13 26 48 / .12);
    border-radius: 16px;
    background: white;
    box-shadow: 0 32px 70px rgb(26 65 120 / .28);
    transform: rotate(-3deg);
  }
  .window-bar {
    min-height: 43px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 15px;
    border-bottom: 1px solid var(--m-border);
    color: var(--m-faint);
    font-size: 9px;
  }
  .window-bar > span { width: 7px; height: 7px; border-radius: 50%; background: #d4d4d2; }
  .window-bar strong { margin-left: 8px; color: var(--m-ink); font-size: 10px; }
  .window-bar small { margin-left: auto; color: #16845b; font-size: 8px; }
  .agent-main { padding: 22px; }
  .agent-heading {
    display: grid;
    grid-template-columns: 38px 1fr auto;
    align-items: center;
    gap: 11px;
    padding-bottom: 22px;
  }
  .agent-heading > span {
    width: 38px;
    height: 38px;
    display: grid;
    place-items: center;
    border-radius: 10px;
    background: var(--m-violet);
    color: white;
  }
  .agent-heading div { display: grid; }
  .agent-heading small { color: var(--m-faint); font-size: 8px; text-transform: uppercase; letter-spacing: .04em; }
  .agent-heading strong { margin-top: 2px; font-size: 13px; }
  .agent-heading i { width: 8px; height: 8px; border-radius: 50%; background: var(--m-green); box-shadow: 0 0 0 4px rgb(0 168 107 / .12); }
  .printer-row {
    min-height: 66px;
    display: grid;
    grid-template-columns: 35px 1fr auto;
    align-items: center;
    gap: 10px;
    padding: 10px 0;
    border-top: 1px solid var(--m-border);
  }
  .printer-row > span {
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border-radius: 9px;
    background: #f4f4f2;
    color: var(--m-muted);
  }
  .printer-row div { display: grid; }
  .printer-row strong { font-size: 11px; }
  .printer-row small { color: var(--m-faint); font-size: 8px; }
  .printer-row b { color: #16845b; font-size: 8px; font-weight: 650; }
  .print-sheet {
    position: absolute;
    z-index: 1;
    width: 126px;
    height: 164px;
    padding: 22px 18px;
    border-radius: 7px;
    background: white;
    box-shadow: 0 20px 40px rgb(32 61 104 / .18);
  }
  .print-sheet span { display: block; width: 52px; height: 52px; border: 9px solid #111; }
  .print-sheet i { display: block; height: 5px; margin-top: 14px; background: #d9d9d7; }
  .print-sheet i:last-child { width: 65%; margin-top: 7px; }
  .sheet-one { top: 35px; right: -18px; transform: rotate(12deg); }
  .sheet-two { left: -22px; bottom: 32px; transform: rotate(-13deg); opacity: .75; }
  .platforms { padding-block: clamp(78px, 10vw, 126px); scroll-margin-top: 80px; }
  .section-heading { max-width: 690px; }
  .section-heading .m-eyebrow { margin-bottom: 18px; }
  .section-heading h2 {
    margin: 0;
    font-family: var(--m-font-editorial);
    font-size: clamp(38px, 5vw, 62px);
    font-weight: 400;
    letter-spacing: -.045em;
    line-height: 1;
    text-wrap: balance;
  }
  .section-heading > p {
    margin: 20px 0 0;
    color: var(--m-muted);
    font-size: 17px;
  }
  .platform-list {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    margin-top: 58px;
    border-top: 1px solid var(--m-border);
    border-bottom: 1px solid var(--m-border);
  }
  .platform-list article {
    min-width: 0;
    min-height: 260px;
    display: grid;
    grid-template-rows: auto 1fr auto;
    align-items: start;
    padding: 30px 32px 28px;
    border-right: 1px solid var(--m-border);
  }
  .platform-list article:first-child { border-left: 1px solid var(--m-border); }
  .platform-list article.detected { background: rgb(255 255 255 / .58); }
  .platform-icon {
    width: 48px;
    height: 48px;
    display: grid;
    place-items: center;
    border: 1px solid var(--m-border);
    border-radius: 13px;
    background: white;
    color: var(--m-violet);
  }
  .platform-copy { align-self: end; margin-top: 30px; }
  .platform-copy > div { display: flex; align-items: center; gap: 9px; }
  .platform-copy h3 { margin: 0; font-size: 22px; letter-spacing: -.035em; }
  .you-are-here {
    padding: 4px 7px;
    border-radius: 999px;
    background: var(--m-violet-soft);
    color: var(--m-violet-dark);
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: .04em;
  }
  .platform-copy p { min-height: 42px; margin: 9px 0 0; color: var(--m-muted); font-size: 13px; line-height: 1.45; }
  .platform-copy small { color: var(--m-faint); font-size: 10px; text-transform: uppercase; letter-spacing: .035em; }
  .platform-copy .platform-warning {
    min-height: 0;
    margin: 9px 0 0;
    color: #7a5000;
    font-size: 11px;
    font-weight: 650;
  }
  .platform-action {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    margin-top: 25px;
    color: var(--m-ink);
    font-size: 13px;
    font-weight: 700;
  }
  .platform-action:hover { color: var(--m-violet-dark); }
  .setup {
    padding-block: clamp(80px, 11vw, 140px);
    background: var(--m-dark);
    color: white;
  }
  .setup .m-eyebrow,
  .setup h2 { color: white; }
  .steps {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 0;
    margin: 64px 0 0;
    padding: 0;
    border-top: 1px solid var(--m-border-light);
    list-style: none;
  }
  .steps li {
    min-height: 250px;
    padding: 31px 34px 30px 0;
    border-right: 1px solid var(--m-border-light);
  }
  .steps li + li { padding-left: 34px; }
  .steps li:last-child { border-right: 0; }
  .steps li > span {
    display: block;
    margin-bottom: 64px;
    color: #6aa8ff;
    font: 12px var(--font-mono);
  }
  .steps h3 { margin: 0; font-size: 23px; font-weight: 580; letter-spacing: -.035em; }
  .steps p { max-width: 300px; margin: 12px 0 0; color: #aeadb1; font-size: 14px; line-height: 1.55; }
  .setup-actions { display: flex; flex-wrap: wrap; gap: 10px; margin-top: 48px; }
  .setup .m-button.dark { border-color: white; background: white; color: var(--m-dark); }
  .setup .m-button:not(.dark) { border-color: var(--m-border-light); background: transparent; color: white; }
  .release-details { padding-block: clamp(80px, 10vw, 126px); }
  .details-list { margin-top: 50px; border-top: 1px solid var(--m-border); }
  details { border-bottom: 1px solid var(--m-border); }
  summary {
    min-height: 82px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    cursor: pointer;
    list-style: none;
  }
  summary::-webkit-details-marker { display: none; }
  summary > span { display: flex; align-items: baseline; gap: 12px; }
  summary strong { font-size: 17px; }
  summary small { color: var(--m-faint); font-size: 11px; text-transform: uppercase; letter-spacing: .04em; }
  summary :global(svg) { transition: transform 180ms ease; }
  details[open] summary :global(svg) { transform: rotate(180deg); }
  .detail-content { padding: 0 0 34px; }
  .status-reason { max-width: 660px; margin: 0 0 24px; color: var(--m-muted); }
  dl { display: grid; grid-template-columns: 1fr 1fr; gap: 0 32px; margin: 0; }
  dl > div {
    min-width: 0;
    display: grid;
    grid-template-columns: 110px minmax(0, 1fr);
    gap: 18px;
    padding: 12px 0;
    border-top: 1px solid var(--m-border);
  }
  dt { color: var(--m-faint); font-size: 11px; }
  dd { min-width: 0; margin: 0; color: var(--m-muted); font-size: 11px; text-align: right; }
  dd code { display: block; overflow-wrap: anywhere; font: 9px/1.5 var(--font-mono); }
  dd a { color: var(--m-violet-dark); text-decoration: underline; }
  .detail-content ul { margin: 22px 0 0; padding-left: 18px; color: var(--m-muted); font-size: 12px; }
  .detail-links { display: flex; gap: 22px; margin-top: 24px; }
  .detail-links a {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--m-ink);
    font-size: 12px;
    font-weight: 680;
  }
  .detail-links a:hover { color: var(--m-violet-dark); }
  .history {
    display: flex;
    justify-content: space-between;
    gap: 20px;
    padding-top: 28px;
    color: var(--m-faint);
    font-size: 10px;
    text-transform: capitalize;
  }
  .history a { display: inline-flex; align-items: center; gap: 6px; color: var(--m-ink); font-weight: 650; }
  @media (max-width: 800px) {
    .hero-inner {
      min-height: 0;
      grid-template-columns: 1fr;
      gap: 54px;
      padding-block: 70px;
    }
    .hero-copy-panel { max-width: 650px; }
    .hero-visual { min-height: 520px; }
    .platform-list { grid-template-columns: 1fr; }
    .platform-list article {
      min-height: 0;
      grid-template-columns: auto 1fr auto;
      grid-template-rows: auto;
      align-items: center;
      gap: 18px;
      padding: 24px 0;
      border-right: 0;
      border-bottom: 1px solid var(--m-border);
    }
    .platform-list article:first-child { border-left: 0; }
    .platform-list article:last-child { border-bottom: 0; }
    .platform-list article.detected { background: transparent; }
    .platform-copy { align-self: auto; margin-top: 0; }
    .platform-copy p { min-height: 0; }
    .platform-action { margin-top: 0; }
    .steps { grid-template-columns: 1fr; }
    .steps li,
    .steps li + li {
      min-height: 0;
      padding: 29px 0;
      border-right: 0;
      border-bottom: 1px solid var(--m-border-light);
    }
    .steps li:last-child { border-bottom: 0; }
    .steps li > span { margin-bottom: 35px; }
    dl { grid-template-columns: 1fr; }
  }
  @media (max-width: 560px) {
    .hero-inner { gap: 42px; padding-block: 58px; }
    h1 { font-size: clamp(43px, 13vw, 62px); }
    .hero-copy { font-size: 16px; }
    .hero-actions { width: 100%; display: grid; justify-items: stretch; }
    .text-link { text-align: center; }
    .download-button { width: 100%; min-height: 54px !important; padding-inline: 18px !important; font-size: 14px !important; }
    .hero-visual { min-height: 400px; border-radius: 16px; }
    .agent-window { width: 88%; }
    .agent-main { padding: 16px; }
    .print-sheet { width: 90px; height: 120px; padding: 15px 13px; }
    .print-sheet span { width: 38px; height: 38px; border-width: 7px; }
    .platform-list article { grid-template-columns: auto 1fr; }
    .platform-action { grid-column: 2; }
    .platform-copy p { margin-right: 0; }
    .setup-actions { display: grid; }
    summary > span { display: grid; gap: 3px; }
    .detail-content dl > div { grid-template-columns: 90px minmax(0, 1fr); }
    .history { display: grid; }
  }
  @media (prefers-reduced-motion: reduce) {
    .download-hero { scroll-behavior: auto; }
  }
</style>
