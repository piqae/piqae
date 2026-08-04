<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import { printNodePricingReviewDueAt } from '$lib/marketing/calculator';

  const claimsExpired = new Date() > new Date(`${printNodePricingReviewDueAt}T23:59:59Z`);

  const phases = [
    ['Inventory', 'Record the endpoints, credential scopes, printers, formats, options, status dependencies, and retry policy your current integration actually uses.'],
    ['Map', 'Keep compatible request shapes, change the base URL and key, then map only the behaviours outside Piqae’s documented subset.'],
    ['Prepare', 'Enrol a Piqae node, resolve its printer and profile IDs, create a scoped key, and define success and rollback thresholds.'],
    ['Canary', 'Route a bounded, non-critical job set. Compare API responses, native queue results, webhooks, and operational recovery.'],
    ['Expand', 'Increase traffic in stages while preserving the old base URL and credentials for immediate rollback.'],
    ['Close', 'Retire the prior path only after a complete operating cycle, incident drill, and evidence review.']
  ];
</script>

<Seo
  title="Migrate from PrintNode to Piqae — Keep the integration"
  description="Move a supported PrintNode integration to Piqae with a base URL and key change, then canary printers, verify statuses, and preserve rollback."
  path="/migrate/printnode"
  noindex={claimsExpired}
/>

<MarketingShell announcement={claimsExpired ? 'Comparison evidence is past its review date and excluded from search' : undefined}>
  <section class="m-page-hero">
    <div class="m-container">
      <span class="m-eyebrow">PrintNode migration</span>
      <h1 class="m-title">Keep the request shape. Change who owns the print path.</h1>
      <p class="m-lede">
        For the documented compatibility subset, move by changing the base URL and API key—not by
        rebuilding your integration. Then canary real printer profiles before moving production
        traffic to Piqae’s managed durable queue.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="/docs/legacy-compatibility">Open technical documentation</a>
        <a class="m-button" href="/compare/printnode">Compare products first</a>
      </div>
    </div>
  </section>

  <section class="matrix m-section-compact">
    <div class="m-container">
      <span class="m-eyebrow">Compatibility envelope</span>
      <div class="m-table-wrap">
        <table class="m-table">
          <thead><tr><th>Surface</th><th>V1 posture</th><th>Migration action</th></tr></thead>
          <tbody>
            <tr><td>Computers / nodes</td><td>Compatible V1 subset</td><td>Map identity and online semantics</td></tr>
            <tr><td>Printers</td><td>Compatible V1 subset</td><td>Resolve IDs from enrolled Piqae nodes</td></tr>
            <tr><td>Print jobs</td><td>Compatible V1 subset</td><td>Keep supported request shapes and test state fixtures</td></tr>
            <tr><td>whoami, ping, noop</td><td>Covered subset</td><td>Run client health checks</td></tr>
            <tr><td>Scales</td><td>Not claimed</td><td>Keep existing path or build separately</td></tr>
            <tr><td>Integrator subaccount APIs</td><td>Not wire-compatible</td><td>Map to Piqae platform customers and service accounts</td></tr>
            <tr><td>Historical response quirks</td><td>Not universally replicated</td><td>Fixture-test your exact client</td></tr>
          </tbody>
        </table>
      </div>
      <p class="m-source">
        Scope reflects the checked-in V1 execution ledger. PrintNode’s original API is documented at
        <a href="https://www.printnode.com/en/docs/api/curl">printnode.com</a>. Verified 30 July 2026.
      </p>
    </div>
  </section>

  <section class="phases m-section">
    <div class="m-container">
      <span class="m-eyebrow">Migration runbook</span>
      {#each phases as phase, index}
        <article>
          <span>{String(index + 1).padStart(2, '0')}</span>
          <h2>{phase[0]}</h2>
          <p>{phase[1]}</p>
        </article>
      {/each}
    </div>
  </section>

  <section class="rollback m-section">
    <div class="m-container m-grid-2">
      <div>
        <span class="m-eyebrow">Rollback contract</span>
        <h2 class="m-heading">Define “stop” before the canary begins.</h2>
      </div>
      <div>
        <ul class="m-list">
          <li>Keep the prior base URL and credentials available during the migration window.</li>
          <li>Stop expansion on duplicate risk, unexplained uncertainty, or unsupported driver options.</li>
          <li>Never automatically reprint an ambiguous job while rolling traffic back.</li>
          <li>Record the final disposition of every canary job across both systems.</li>
        </ul>
      </div>
    </div>
  </section>

  <section class="why m-section">
    <div class="m-container m-grid-3">
      <article class="m-card">
        <span>Commercial</span>
        <h2>Only reported-complete jobs count.</h2>
        <p>
          Jobs that remain failed, jammed, blocked, cancelled, expired, or delivery-uncertain do
          not consume Piqae usage. A recovered job counts only if it reaches reported complete,
          which remains a device or OS signal—not physical proof.
        </p>
      </article>
      <article class="m-card">
        <span>Operational</span>
        <h2>Offline nodes can receive the job later.</h2>
        <p>
          The managed queue retains durable work for reconnection while the local node maintains
          its own durable handoff state. Self-hosters keep the model and operate its availability.
        </p>
      </article>
      <article class="m-card">
        <span>Strategic</span>
        <h2>The new edge is Apache-2.0.</h2>
        <p>
          Audit it, adapt it, or preserve a self-hosted exit. Product-specific distribution remains
          subject to code-signing, update-policy, licence, and trademark requirements.
        </p>
      </article>
    </div>
  </section>
</MarketingShell>

<style>
  .matrix { background: #eeece6; }
  .m-source { margin-top: 15px; }
  .phases article {
    display: grid; grid-template-columns: 80px 220px 1fr; gap: 36px;
    padding: 34px 0; border-top: 1px solid var(--m-border);
  }
  .phases article > span { color: var(--m-violet-dark); font: 11px var(--font-mono); }
  .phases h2 { margin: 0; font-size: 25px; letter-spacing: -.04em; }
  .phases p { max-width: 660px; margin: 0; color: var(--m-muted); }
  .rollback { background: #eeece6; }
  .why { border-top: 1px solid var(--m-border); }
  .why article > span { color: var(--m-violet-dark); font-size: 11px; font-weight: 700; text-transform: uppercase; }
  .why h2 { margin-top: 46px; font-size: 24px; letter-spacing: -.04em; }
  .why p { color: var(--m-muted); }
  @media (max-width: 700px) {
    .phases article { grid-template-columns: 1fr; gap: 10px; }
  }
</style>
