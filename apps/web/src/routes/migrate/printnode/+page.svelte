<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import { printNodePricingReviewDueAt } from '$lib/marketing/calculator';

  const claimsExpired = new Date() > new Date(`${printNodePricingReviewDueAt}T23:59:59Z`);

  const phases = [
    ['Inventory', 'Record every endpoint, credential, printer, content format, option, status dependency, and retry policy your current integration uses.'],
    ['Map', 'Compare those behaviours with Spool’s tested compatibility matrix. Treat gaps as explicit work, not assumptions.'],
    ['Prepare', 'Enroll a separate agent, map printer identities, create scoped credentials, and define success and rollback thresholds.'],
    ['Canary', 'Route a bounded, non-critical job set. Compare API responses, native queue results, webhooks, and operational recovery.'],
    ['Expand', 'Increase traffic in stages while preserving the old base URL and credentials for immediate rollback.'],
    ['Close', 'Retire the prior path only after a complete operating cycle, incident drill, and evidence review.']
  ];
</script>

<Seo
  title="Migrate from PrintNode to Spool"
  description="A staged PrintNode migration guide covering endpoint inventory, compatibility mapping, printer canaries, observability, and rollback."
  path="/migrate/printnode"
  noindex={claimsExpired}
/>

<MarketingShell announcement={claimsExpired ? 'Comparison evidence is past its review date and excluded from search' : undefined}>
  <section class="m-page-hero">
    <div class="m-container">
      <span class="m-eyebrow">PrintNode migration</span>
      <h1 class="m-title">Change the print path without betting the operation.</h1>
      <p class="m-lede">
        Spool supports a migration-critical subset of the PrintNode printing API. Use that
        compatibility to canary deliberately—not to assume every historical behaviour matches.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="/docs/printnode-migration">Open technical documentation</a>
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
            <tr><td>Computers / nodes</td><td>Migration-critical subset</td><td>Map identity and online semantics</td></tr>
            <tr><td>Printers</td><td>Migration-critical subset</td><td>Re-resolve IDs from enrolled agents</td></tr>
            <tr><td>Print jobs</td><td>Migration-critical subset</td><td>Test request and state fixtures</td></tr>
            <tr><td>whoami, ping, noop</td><td>Covered subset</td><td>Run client health checks</td></tr>
            <tr><td>Scales</td><td>Not claimed</td><td>Keep existing path or build separately</td></tr>
            <tr><td>Integrator subaccount APIs</td><td>Not claimed as compatible</td><td>Map to Spool workspaces explicitly</td></tr>
            <tr><td>Historical response quirks</td><td>Not universally replicated</td><td>Fixture-test your exact client</td></tr>
          </tbody>
        </table>
      </div>
      <p class="m-source">
        Scope reflects the checked-in V1 execution ledger. PrintNode’s original API is documented at
        <a href="https://www.printnode.com/en/docs/api/curl">printnode.com</a>. Verified 29 July 2026.
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
  @media (max-width: 700px) {
    .phases article { grid-template-columns: 1fr; gap: 10px; }
  }
</style>
