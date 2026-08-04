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
  title="Piqae vs PrintNode — Open, lower-cost remote printing"
  description="Compare Piqae and PrintNode across reported-complete usage, deployment, open source access, migration, white-label flexibility, and release evidence."
  path="/compare/printnode"
  noindex={claimsExpired}
  structuredData={{
    '@context': 'https://schema.org',
    '@type': 'BreadcrumbList',
    itemListElement: [
      { '@type': 'ListItem', position: 1, name: 'Home', item: '/' },
      { '@type': 'ListItem', position: 2, name: 'Compare', item: '/compare' },
      { '@type': 'ListItem', position: 3, name: 'Piqae vs PrintNode', item: '/compare/printnode' }
    ]
  }}
/>

<MarketingShell announcement={claimsExpired ? 'Comparison evidence is past its review date and excluded from search' : undefined}>
  <ComparisonHero
    eyebrow="Piqae vs PrintNode"
    title="A familiar print API with a more open operating model."
    description="Piqae combines an Apache-2.0 edge, managed durable queue, completion-based usage, and a self-hosted exit. PrintNode remains the more established service with mature desktop coverage."
    verified="30 July 2026"
    source="https://www.printnode.com/en"
  />

  <section class="best-for m-section-compact">
    <div class="m-container m-grid-2">
      <article class="m-card">
        <span>Piqae may fit better when</span>
        <h2>You want less integration and more control.</h2>
        <ul class="m-list">
          <li>You want lower list pricing and only reported-complete jobs to count.</li>
          <li>Self-hosting, local-only operation, or an open-source exit is required.</li>
          <li>You need durable Cloud and local queues with explicit uncertain delivery.</li>
          <li>You are embedding multi-tenant printing or planning a product-specific node.</li>
        </ul>
      </article>
      <article class="m-card">
        <span>PrintNode may fit better when</span>
        <h2>You need a mature hosted client today.</h2>
        <ul class="m-list">
          <li>You require its current signed desktop coverage and production history.</li>
          <li>You depend on scales or historical API behaviours outside Piqae’s tested subset.</li>
          <li>You prefer its established hosted operation or contact-sales Standalone Server.</li>
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
          <thead><tr><th>Area</th><th>Piqae</th><th>PrintNode</th></tr></thead>
          <tbody>
            <tr><td>Source model</td><td>Apache-2.0 node, control plane, web app, and SDK</td><td>Proprietary service and clients</td></tr>
            <tr><td>Deployment</td><td>Managed Cloud, complete self-hosting, or local-only</td><td>PrintNode Cloud or contact-sales Standalone Server, with local clients</td></tr>
            <tr><td>Local software</td><td>Native Rust agent with thin OS shells</td><td>Desktop client and service options vary by OS</td></tr>
            <tr><td>Formats</td><td>PDF and RAW in the tested V1 envelope</td><td>PDF and RAW, plus documented platform behaviours</td></tr>
            <tr><td>Driver options</td><td>Installed OS drivers remain authoritative</td><td>Client exposes documented printer capabilities</td></tr>
            <tr><td>Status position</td><td>Accepted, reported complete, and uncertain remain distinct</td><td>Documented print-job states through the hosted API</td></tr>
            <tr><td>Billable unit</td><td>One job reported complete by the node; jobs that remain failed, jammed, blocked, cancelled, or uncertain do not count</td><td>One API print request, regardless of number of pages or print outcome</td></tr>
            <tr><td>Product embedding</td><td>Platform accounts, workspace isolation, open edge, and custom distribution path</td><td>Branding and account-provisioning APIs are documented</td></tr>
            <tr><td>Migration API</td><td>Tested subset for computers, printers, jobs, states, whoami, ping, noop</td><td>Original API surface</td></tr>
            <tr><td>Availability posture</td><td>99.95% architecture design target; no contractual beta SLA</td><td>Established hosted production service</td></tr>
            <tr><td>Current native release</td><td>Signed and notarised macOS preview; Windows remains behind release gates</td><td>Mature signed desktop clients</td></tr>
          </tbody>
        </table>
      </div>
      <p class="m-source">
        PrintNode facts: <a href="https://www.printnode.com/en/features">features</a>,
        <a href="https://www.printnode.com/en/faq">Standalone Server FAQ</a>,
        <a href="https://www.printnode.com/en/download">downloads</a>, and
        <a href="https://www.printnode.com/en/docs/api/curl">API documentation</a>.
        Piqae status comes from the repository support matrix. Checked 30 July 2026.
      </p>
    </div>
  </section>

  <section class="pricing m-section">
    <div class="m-container">
      <span class="m-eyebrow">Public list pricing</span>
      <h2 class="m-heading">Pay less—and only when the node reports completion.</h2>
      <p class="intro">
        Piqae Pro lists at $9 per month for 25,000 reported-complete jobs and up to 25 nodes.
        PrintNode Standard lists at $29 per month for 25,000 API print requests and five
        computers. PrintNode says a request counts as a print regardless of its outcome.
      </p>
      <div class="m-table-wrap">
        <table class="m-table">
          <thead><tr><th>Example tier</th><th>Piqae displayed price</th><th>PrintNode public price</th></tr></thead>
          <tbody>
            {#each data.pricing.plans as plan}
              <tr>
                <td>{plan.name}</td>
                <td>
                  {plan.name} · {formatUsd(plan.monthlyCents)} ·
                  {plan.includedReportedCompleteJobs.toLocaleString('en-US')} reported-complete jobs ·
                  {plan.includedNodes} {plan.includedNodes === 1 ? 'node' : 'nodes'}
                </td>
                <td>
                  {plan.plan === 'free'
                    ? 'Lite · $0 · 50 API print requests · 1 computer'
                    : 'Standard · $29 · 25k API print requests · 5 computers'}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="m-source">
        PrintNode USD list prices from <a href="https://www.printnode.com/en/pricing">its pricing page</a>,
        checked 30 July 2026. PrintNode defines a print as one API print request regardless of
        pages or outcome. Taxes, negotiated terms, and overages can change the result. Piqae values
        come from server catalog {data.pricing.version}; checkout must match that catalog.
      </p>
      <p class="m-source">
        “Reported complete” is the strongest completion signal returned by the node, operating
        system, driver, or printer. It is the Piqae billable event, but it is not proof that ink
        physically reached paper when the hardware cannot provide that evidence. A blocked or
        jammed job that later recovers and reaches reported complete does count.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="/tools/printnode-cost-calculator">Model your usage</a>
        <a class="m-button" href="/migrate/printnode">Plan a migration</a>
      </div>
    </div>
  </section>

  <section class="cloud m-section">
    <div class="m-container m-grid-2">
      <div>
        <span class="m-eyebrow">Managed without lock-in</span>
        <h2 class="m-heading">Customise the edge. Keep the operated queue.</h2>
      </div>
      <div class="m-prose">
        <p>
          Piqae Cloud is the low-operations path: we run the control plane, durable document
          storage, queue recovery, monitoring, backups, and upgrades. The Apache-2.0 node remains
          auditable and can be adapted for a product-specific distribution, subject to licence,
          trademark, signing, and update-policy requirements.
        </p>
        <p>
          Self-hosting keeps the complete printing model, but your team becomes responsible for
          database and object-store durability, monitoring, backups, failover, and upgrades.
        </p>
      </div>
    </div>
  </section>

  <section class="limits m-section">
    <div class="m-narrow m-prose">
      <h2>What the current release evidence says.</h2>
      <p>
        The macOS package is signed and notarised but remains preview support. Windows packaging
        and physical certification remain behind checked-in release gates. Use controlled canaries
        until the platform and printer combination you depend on is certified.
      </p>
      <h2>The compatibility promise is specific.</h2>
      <p>
        Piqae covers the documented migration-critical subset, so supported integrations can
        change the base URL and credential rather than rewrite their print path. Scales, PrintNode
        billing APIs, and every historical response quirk are not claimed as compatible.
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
  .cloud { border-top: 1px solid var(--m-border); }
  .limits { border-top: 1px solid var(--m-border); background: #eeece6; }
</style>
