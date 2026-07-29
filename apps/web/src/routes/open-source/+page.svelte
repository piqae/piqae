<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
</script>

<Seo
  title="Open-source printing infrastructure — Spool"
  description="Audit, modify, and self-host Spool under the Apache-2.0 licence. Keep local and self-hosted printing first-class."
  path="/open-source"
/>

<MarketingShell>
  <section class="m-page-hero">
    <div class="m-container">
      <span class="m-eyebrow">Apache-2.0</span>
      <h1 class="m-title">Open by design. Yours to run.</h1>
      <p class="m-lede">
        The agent, control plane, SDK, dashboard, and deployment definitions are open source.
        Choose managed Cloud for convenience or run the complete platform in your own environment.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="https://github.com/C4CoffeeCo/spool">View on GitHub</a>
        <a class="m-button" href="/docs/self-hosting">Self-hosting guide</a>
      </div>
    </div>
  </section>

  <section class="principles m-section">
    <div class="m-container m-grid-3">
      <article class="m-card"><span>01</span><h2>Local stays real</h2><p>The product model must work without a managed cloud control plane.</p></article>
      <article class="m-card"><span>02</span><h2>The edge is auditable</h2><p>Inspect the code that downloads content, stores a queue, and calls native print APIs.</p></article>
      <article class="m-card"><span>03</span><h2>Drivers keep authority</h2><p>Vendor options come from the operating system, not an opaque remote approximation.</p></article>
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
      <pre aria-label="Repository architecture"><code>spool/
├── control-plane
├── agent-core
├── spool-agent
├── protocol
├── storage-postgres
├── web
├── sdk
└── native shells</code></pre>
    </div>
  </section>

  <section class="license m-section">
    <div class="m-container m-dark-panel">
      <div>
        <span>Apache License 2.0</span>
        <h2>Use it commercially. Modify it. Operate it yourself.</h2>
        <p>The licence file in the repository is authoritative; this page is not legal advice.</p>
      </div>
      <a class="m-button primary" href="https://github.com/C4CoffeeCo/spool/blob/main/LICENSE">Read the licence</a>
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
  .license { background: #eeece6; }
  .license .m-dark-panel {
    display: flex; align-items: end; justify-content: space-between; gap: 30px;
    padding: clamp(34px, 6vw, 68px);
  }
  .license span { color: #71adff; font-size: 12px; text-transform: uppercase; }
  .license h2 { max-width: 730px; margin: 12px 0; font-size: clamp(32px, 5vw, 58px); letter-spacing: -.055em; }
  .license p { margin: 0; }
  @media (max-width: 760px) {
    .architecture { grid-template-columns: 1fr; }
    .license .m-dark-panel { align-items: flex-start; flex-direction: column; }
  }
</style>
