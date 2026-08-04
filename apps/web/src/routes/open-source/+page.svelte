<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
</script>

<Seo
  title="Open-source printing infrastructure — Piqae"
  description="Audit, modify, and self-host Piqae under the Apache-2.0 licence. Keep local and self-hosted printing first-class."
  path="/open-source"
/>

<MarketingShell>
  <section class="m-page-hero">
    <div class="m-container">
      <span class="m-eyebrow">Apache-2.0</span>
      <h1 class="m-title">Open at the edge. Portable at the core.</h1>
      <p class="m-lede">
        The node, control plane, SDK, dashboard, and deployment definitions are open source.
        Run the complete platform yourself, or keep an auditable, customisable edge while Piqae
        Cloud operates the durable queue.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="https://github.com/piqae/piqae">View on GitHub</a>
        <a class="m-button" href="/docs/self-hosting">Self-hosting guide</a>
      </div>
    </div>
  </section>

  <section class="principles m-section">
    <div class="m-container m-grid-3">
      <article class="m-card"><span>01</span><h2>Local stays real</h2><p>Printing, native profiles, and the local queue do not become artificial Cloud-only features.</p></article>
      <article class="m-card"><span>02</span><h2>The edge is yours</h2><p>Audit or adapt the code that stores jobs, reconnects, and calls native print APIs.</p></article>
      <article class="m-card"><span>03</span><h2>Drivers keep authority</h2><p>Vendor options come from the operating system, including specialist label and PostScript drivers.</p></article>
    </div>
  </section>

  <section class="m-section">
    <div class="m-container architecture">
      <div>
        <span class="m-eyebrow">Repository architecture</span>
        <h2 class="m-heading">One shared Rust core. Thin native shells.</h2>
        <p>
          The durable agent owns identity, queueing, recovery, and cloud synchronisation. Native
          menu and tray applications remain disposable interfaces over a narrow local contract.
        </p>
      </div>
      <pre aria-label="Repository architecture"><code>piqae/
├── control-plane
├── agent-core
├── piqae-agent
├── protocol
├── storage-postgres
├── web
├── sdk
└── native shells</code></pre>
    </div>
  </section>

  <section class="operations m-section">
    <div class="m-container">
      <div class="operations-heading">
        <div>
          <span class="m-eyebrow">Choose who operates it</span>
          <h2 class="m-heading">Open source without giving up a managed path.</h2>
        </div>
        <p>
          Both routes use the same product model. The difference is who owns the stateful
          infrastructure and the work required to keep it available.
        </p>
      </div>
      <div class="m-grid-2">
        <article class="m-card">
          <span>Piqae Cloud</span>
          <h3>Keep the open edge. Hand us the queue.</h3>
          <p>
            Piqae operates the control plane, object storage, backups, monitoring, upgrades, and
            recovery path. This is the simplest route when print jobs must wait safely for offline
            nodes without your team running printing infrastructure.
          </p>
        </article>
        <article class="m-card">
          <span>Self-hosted</span>
          <h3>Run the complete platform yourself.</h3>
          <p>
            Compose and Kubernetes deployments keep core printing features available, but your
            team owns database durability, storage, scaling, monitoring, backups, and high
            availability.
          </p>
        </article>
      </div>
    </div>
  </section>

  <section class="license m-section-compact">
    <div class="m-container license-card">
      <div class="license-intro">
        <span class="m-eyebrow">Apache License 2.0</span>
        <h2>Built to be used, changed, and redistributed.</h2>
        <p>
          Use Piqae commercially, inspect every handoff, maintain a compatible fork, or operate it
          in your own environment.
        </p>
      </div>
      <ul aria-label="Apache-2.0 licence summary">
        <li><strong>Use</strong><span>Personal or commercial</span></li>
        <li><strong>Modify</strong><span>Adapt the node or platform</span></li>
        <li><strong>Distribute</strong><span>Subject to licence notices</span></li>
        <li><strong>Self-host</strong><span>No licence server or phone-home</span></li>
      </ul>
      <div class="license-footer">
        <p>
          Piqae trademarks are separate from the source licence. The repository licence and
          trademark policy are authoritative; this summary is not legal advice.
        </p>
        <div class="m-actions">
          <a class="m-button primary" href="https://github.com/piqae/piqae/blob/main/LICENSE">Read the licence</a>
          <a class="m-button" href="https://github.com/piqae/piqae/blob/main/TRADEMARKS.md">Trademark policy</a>
        </div>
      </div>
    </div>
  </section>
</MarketingShell>

<style>
  .principles { background: #eeece6; }
  .principles article { min-height: 270px; }
  .principles article > span { color: var(--m-violet-dark); font: 11px var(--font-mono); }
  .principles h2 { margin-top: 90px; font-size: 21px; }
  .architecture { display: grid; grid-template-columns: 1fr .8fr; gap: 70px; align-items: center; }
  .architecture p { color: var(--m-muted); font-size: 17px; }
  pre {
    margin: 0;
    padding: 30px;
    border: 1px solid var(--m-border);
    border-radius: 16px;
    background: white;
    color: var(--m-ink);
    font: 13px/1.9 var(--font-mono);
  }
  .operations { background: #eeece6; }
  .operations-heading {
    display: grid;
    grid-template-columns: 1fr .8fr;
    gap: 70px;
    align-items: end;
    margin-bottom: 34px;
  }
  .operations-heading p, .operations article p { color: var(--m-muted); }
  .operations article > span {
    color: var(--m-violet-dark);
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
  }
  .operations h3 { margin: 44px 0 12px; font-size: 24px; letter-spacing: -.04em; }
  .license { background: #f7f6f2; }
  .license-card {
    overflow: hidden;
    border: 1px solid var(--m-border);
    border-radius: 18px;
    background: white;
  }
  .license-intro {
    display: grid;
    grid-template-columns: .65fr 1fr;
    gap: 50px;
    padding: clamp(26px, 4vw, 44px);
  }
  .license-intro h2 {
    grid-column: 1;
    max-width: 460px;
    margin: 7px 0 0;
    font-size: clamp(28px, 4vw, 44px);
    letter-spacing: -.05em;
  }
  .license-intro p {
    grid-column: 2;
    grid-row: 1 / span 2;
    align-self: end;
    max-width: 570px;
    margin: 0;
    color: var(--m-muted);
  }
  .license ul {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    margin: 0;
    padding: 0;
    border-block: 1px solid var(--m-border);
    list-style: none;
  }
  .license li {
    display: flex;
    min-height: 98px;
    flex-direction: column;
    justify-content: center;
    gap: 6px;
    padding: 20px 24px;
    border-right: 1px solid var(--m-border);
  }
  .license li:last-child { border-right: 0; }
  .license li strong { font-size: 14px; }
  .license li span { color: var(--m-faint); font-size: 12px; }
  .license-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 28px;
    padding: 20px 24px;
  }
  .license-footer p { max-width: 610px; margin: 0; color: var(--m-faint); font-size: 12px; }
  @media (max-width: 760px) {
    .architecture { grid-template-columns: 1fr; }
    .operations-heading, .license-intro { grid-template-columns: 1fr; gap: 16px; }
    .license-intro p { grid-column: 1; grid-row: auto; }
    .license ul { grid-template-columns: repeat(2, 1fr); }
    .license li:nth-child(2) { border-right: 0; }
    .license li:nth-child(-n + 2) { border-bottom: 1px solid var(--m-border); }
    .license-footer { align-items: flex-start; flex-direction: column; }
  }
  @media (max-width: 440px) {
    .license ul { grid-template-columns: 1fr; }
    .license li { border-right: 0; border-bottom: 1px solid var(--m-border); }
    .license li:nth-child(2) { border-bottom: 1px solid var(--m-border); }
    .license li:last-child { border-bottom: 0; }
  }
</style>
