<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
</script>

<Seo
  title="Piqae security and trust"
  description="Understand Piqae's tenant boundaries, signed agent identity, durable queues, document handling, and deliberately scoped delivery claims."
  path="/security"
/>

<MarketingShell>
  <section class="m-page-hero">
    <div class="m-container">
      <span class="m-eyebrow">Security and trust</span>
      <h1 class="m-title">Built for the documents your business depends on.</h1>
      <p class="m-lede">
        Piqae protects credentials, gives every enrolled agent its own identity, separates
        customer workspaces, and keeps sensitive print data out of marketing analytics.
      </p>
    </div>
  </section>

  <section class="m-section-compact">
    <div class="m-container trust-grid">
      <article class="m-card"><h2>Tenant isolation</h2><p>Workspace and environment identifiers scope every control-plane operation and storage query.</p></article>
      <article class="m-card"><h2>Device identity</h2><p>Enrolled agents authenticate requests with signed, replay-resistant device credentials.</p></article>
      <article class="m-card"><h2>Bounded content</h2><p>Downloads, URIs, files, render work, and waits are constrained rather than trusted indefinitely.</p></article>
      <article class="m-card"><h2>Secret hygiene</h2><p>API keys, enrollment tokens, device keys, lease capabilities, and documents are excluded from client telemetry.</p></article>
      <article class="m-card"><h2>Auditable events</h2><p>State changes are append-oriented and the reported-complete usage event is idempotent.</p></article>
      <article class="m-card"><h2>Honest status</h2><p>Native spooler acceptance and reported completion are not presented as verified physical delivery.</p></article>
    </div>
  </section>

  <section class="encryption m-section">
    <div class="m-container encryption-grid">
      <div>
        <span class="m-eyebrow">Encryption boundaries</span>
        <h2 class="m-heading">Protection matched to each part of the print path.</h2>
        <p class="m-lede">
          Piqae layers transport, storage, and optional payload encryption without hiding where
          plaintext must exist for the operating system, driver, and printer to do their work.
        </p>
      </div>
      <div class="encryption-levels">
        <article><strong>01 · In transit</strong><h3>Authenticated TLS</h3><p>Remote API, upload, and agent connections use TLS with hostname and trust-root validation.</p></article>
        <article><strong>02 · At rest</strong><h3>Object-level encryption</h3><p>Hosted document content uses a random data-encryption key, with short retention and deletion controls.</p></article>
        <article><strong>03 · Confidential printing Preview</strong><h3>AES-256-GCM per job</h3><p>The SDK can encrypt PDF or RAW content before upload and wrap its one-time key separately for each permitted node.</p></article>
      </div>
      <p class="m-note">
        <strong>Precisely scoped:</strong> confidential printing is a Preview path and is not yet
        an independently audited zero-knowledge or end-to-end encrypted service. Routing metadata
        remains visible, and the destination system must decrypt content to print it.
      </p>
    </div>
  </section>

  <section class="data m-section">
    <div class="m-container">
      <span class="m-eyebrow">Data path</span>
      <div class="m-table-wrap">
        <table class="m-table">
          <thead><tr><th>Data</th><th>Where it belongs</th><th>Control</th></tr></thead>
          <tbody>
            <tr><td>Print content</td><td>Bounded object storage and the enrolled agent</td><td>Retention and deletion policy</td></tr>
            <tr><td>Device credentials</td><td>Protected local secret storage</td><td>Revocable enrolment identity</td></tr>
            <tr><td>Job metadata</td><td>Workspace-scoped database rows</td><td>Role and environment boundaries</td></tr>
            <tr><td>Marketing analytics</td><td>Consent-gated PostHog events</td><td>No document, printer, key, or address fields</td></tr>
          </tbody>
        </table>
      </div>
    </div>
  </section>

  <section class="m-section">
    <div class="m-narrow m-prose">
      <h2>Deployment responsibility</h2>
      <p>
        Piqae Cloud operates the managed control plane. Self-hosted operators are responsible for
        their database, object storage, network, identity configuration, retention, upgrades, and
        backups. The native agent remains inside the printer network in either model.
      </p>
      <h2>Current release envelope</h2>
      <p>
        Source-complete does not mean every platform is generally available. The checked-in
        support matrix and release gates are authoritative; the downloads page renders those
        limits instead of making broader claims.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="/docs/security">Read security documentation</a>
        <a class="m-button" href="https://github.com/piqae/piqae/security">Report a vulnerability</a>
      </div>
    </div>
  </section>
</MarketingShell>

<style>
  .trust-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; }
  .trust-grid article { min-height: 220px; }
  .trust-grid h2 { margin-top: 60px; font-size: 19px; }
  .encryption { background: #07111f; color: white; }
  .encryption-grid { display: grid; grid-template-columns: .8fr 1.2fr; gap: clamp(42px, 8vw, 110px); }
  .encryption .m-eyebrow { color: #71adff; }
  .encryption .m-heading { color: white; }
  .encryption .m-lede { color: #9eacbd; font-size: 18px; }
  .encryption-levels { border-top: 1px solid rgb(255 255 255 / .14); }
  .encryption-levels article { padding: 22px 0; border-bottom: 1px solid rgb(255 255 255 / .14); }
  .encryption-levels strong { color: #71adff; font: 10px var(--font-mono); text-transform: uppercase; }
  .encryption-levels h3 { margin: 10px 0 5px; color: white; font-size: 20px; }
  .encryption-levels p { margin: 0; color: #9eacbd; }
  .encryption .m-note { grid-column: 1 / -1; border-color: rgb(255 255 255 / .14); background: rgb(255 255 255 / .05); color: #9eacbd; }
  .encryption .m-note strong { color: white; }
  .data { background: #eeece6; }
  @media (max-width: 900px) {
    .trust-grid { grid-template-columns: repeat(2, 1fr); }
    .encryption-grid { grid-template-columns: 1fr; }
    .encryption .m-note { grid-column: auto; }
  }
  @media (max-width: 620px) { .trust-grid { grid-template-columns: 1fr; } }
</style>
