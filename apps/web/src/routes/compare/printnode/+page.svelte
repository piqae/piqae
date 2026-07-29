<script lang="ts">
  import ComparisonHero from '$lib/components/marketing/ComparisonHero.svelte';
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import { printNodePricingReviewDueAt } from '$lib/marketing/calculator';
  import { formatUsd } from '$lib/marketing/plans';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();
  const claimsExpired = new Date() > new Date(`${printNodePricingReviewDueAt}T23:59:59Z`);
</script>

<Seo
  title="Spool vs PrintNode — A factual printing API comparison"
  description="Compare Spool and PrintNode across deployment, open source access, API migration, agents, print formats, pricing, and product maturity."
  path="/compare/printnode"
  noindex={claimsExpired}
  structuredData={{
    '@context': 'https://schema.org',
    '@type': 'BreadcrumbList',
    itemListElement: [
      { '@type': 'ListItem', position: 1, name: 'Home', item: '/' },
      { '@type': 'ListItem', position: 2, name: 'Compare', item: '/compare' },
      { '@type': 'ListItem', position: 3, name: 'Spool vs PrintNode', item: '/compare/printnode' }
    ]
  }}
/>

<MarketingShell announcement={claimsExpired ? 'Comparison evidence is past its review date and excluded from search' : undefined}>
  <ComparisonHero
    eyebrow="Spool vs PrintNode"
    title="Two remote print APIs, with different control boundaries."
    description="PrintNode is an established hosted service with mature desktop clients. Spool is an Apache-2.0 alternative built around open deployment, durable edge ownership, and explicit status semantics."
    verified="29 July 2026"
    source="https://www.printnode.com/en"
  />

  <section class="best-for m-section-compact">
    <div class="m-container m-grid-2">
      <article class="m-card">
        <span>Spool may fit better when</span>
        <h2>You need control of the whole print path.</h2>
        <ul class="m-list">
          <li>Self-hosting or local-only operation is a requirement.</li>
          <li>Apache-2.0 source access matters to security or procurement.</li>
          <li>You need explicit durable queues on the server and local agent.</li>
          <li>You are building a multi-tenant printing product.</li>
        </ul>
      </article>
      <article class="m-card">
        <span>PrintNode may fit better when</span>
        <h2>You need a mature hosted client today.</h2>
        <ul class="m-list">
          <li>You require its current signed desktop coverage and production history.</li>
          <li>You depend on scales or historical API behaviours outside Spool’s tested subset.</li>
          <li>You prefer an established hosted vendor over operating open infrastructure.</li>
          <li>Your integration already works and migration has no meaningful return.</li>
        </ul>
      </article>
    </div>
  </section>

  <section class="m-section">
    <div class="m-container">
      <span class="m-eyebrow">Like-for-like view</span>
      <div class="m-table-wrap">
        <table class="m-table">
          <thead><tr><th>Area</th><th>Spool</th><th>PrintNode</th></tr></thead>
          <tbody>
            <tr><td>Source model</td><td>Apache-2.0 open source</td><td>Hosted proprietary service and clients</td></tr>
            <tr><td>Deployment</td><td>Cloud, self-hosted, or local-only</td><td>PrintNode-hosted service with local client</td></tr>
            <tr><td>Local software</td><td>Native Rust agent with thin OS shells</td><td>Desktop client and service options vary by OS</td></tr>
            <tr><td>Formats</td><td>PDF and RAW in the tested V1 envelope</td><td>PDF and RAW, plus documented platform behaviours</td></tr>
            <tr><td>Driver options</td><td>Installed OS drivers remain authoritative</td><td>Client exposes documented printer capabilities</td></tr>
            <tr><td>Status position</td><td>Accepted, reported complete, and uncertain remain distinct</td><td>Documented print-job states through the hosted API</td></tr>
            <tr><td>Migration API</td><td>Tested subset for computers, printers, jobs, states, whoami, ping, noop</td><td>Original API surface</td></tr>
            <tr><td>Current release maturity</td><td>Preview/disabled by checked-in platform gate</td><td>Generally available hosted product</td></tr>
          </tbody>
        </table>
      </div>
      <p class="m-source">
        PrintNode facts: <a href="https://www.printnode.com/en/features">features</a>,
        <a href="https://www.printnode.com/en/download">downloads</a>, and
        <a href="https://www.printnode.com/en/docs/api/curl">API documentation</a>.
        Spool status comes from the repository support matrix. Checked 29 July 2026.
      </p>
    </div>
  </section>

  <section class="pricing m-section">
    <div class="m-container">
      <span class="m-eyebrow">Public list pricing</span>
      <h2 class="m-heading">Compare the unit before comparing the number.</h2>
      <p class="intro">
        Spool has one Free plan and one paid Pro plan. PrintNode uses several job-volume tiers,
        with different computer and subaccount allowances.
      </p>
      <div class="m-table-wrap">
        <table class="m-table">
          <thead><tr><th>Example tier</th><th>Spool displayed price</th><th>PrintNode public price</th></tr></thead>
          <tbody>
            {#each data.pricing.plans as plan}
              <tr>
                <td>{plan.name}</td>
                <td>
                  {plan.name} · {formatUsd(plan.monthlyCents)} ·
                  {plan.includedAcceptedJobs.toLocaleString('en-US')} jobs ·
                  {plan.includedNodes} {plan.includedNodes === 1 ? 'node' : 'nodes'}
                </td>
                <td>
                  {plan.plan === 'free'
                    ? 'Lite · $0 · 50 jobs · 1 computer'
                    : 'Standard · $29 · 25k jobs · 5 computers'}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="m-source">
        PrintNode USD list prices from <a href="https://www.printnode.com/en/pricing">its pricing page</a>,
        checked 29 July 2026. Taxes, negotiated terms, and overages can change the result. Spool
        values come from server catalog {data.pricing.version}; checkout must match that catalog.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="/tools/printnode-cost-calculator">Model your usage</a>
        <a class="m-button" href="/migrate/printnode">Plan a migration</a>
      </div>
    </div>
  </section>

  <section class="limits m-section">
    <div class="m-narrow m-prose">
      <h2>Spool’s current limitation is release maturity.</h2>
      <p>
        The source repository marks Windows and signed packaging disabled and macOS/CUPS
        platforms preview. Until those gates pass, Spool should be evaluated with virtual
        printers and controlled canaries—not described as a production-equivalent replacement.
      </p>
      <h2>Migration compatibility is deliberately scoped.</h2>
      <p>
        Spool covers the migration-critical printing subset documented in its compatibility guide.
        Scales, integrator subaccount APIs, billing APIs, and every historical response quirk are
        not claimed as complete.
      </p>
    </div>
  </section>
</MarketingShell>

<style>
  .best-for { background: #eeece6; }
  .best-for article > span { color: var(--m-violet-dark); font-size: 11px; font-weight: 700; text-transform: uppercase; }
  .best-for h2 { max-width: 440px; margin-top: 50px; font-size: 25px; }
  .m-source { margin-top: 16px; }
  .pricing { background: #eeece6; }
  .intro { max-width: 680px; margin: 18px 0 35px; color: var(--m-muted); }
  .limits { border-top: 1px solid var(--m-border); }
</style>
