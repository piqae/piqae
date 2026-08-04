<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import { printNodePricingReviewDueAt } from '$lib/marketing/calculator';

  const claimsExpired = new Date() > new Date(`${printNodePricingReviewDueAt}T23:59:59Z`);
</script>

<Seo
  title="A lower-cost, open PrintNode alternative — Piqae"
  description="Choose a PrintNode alternative with Apache-2.0 nodes, managed durable queues, reported-complete usage, self-hosting, and platform accounts."
  path="/alternatives/printnode"
  noindex={claimsExpired}
/>

<MarketingShell announcement={claimsExpired ? 'Comparison evidence is past its review date and excluded from search' : undefined}>
  <section class="m-page-hero">
    <div class="m-container">
      <span class="m-eyebrow">PrintNode alternative</span>
      <h1 class="m-title">Remote printing that is easier to adopt—and easier to leave.</h1>
      <p class="m-lede">
        Piqae offers a familiar API, lower list pricing, an Apache-2.0 edge, and a managed durable
        Cloud queue. Use the Cloud without giving up a self-hosted exit or the ability to adapt the
        node for your own product.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="/tools/printnode-cost-calculator">Estimate the cost</a>
        <a class="m-button" href="/compare/printnode">See the factual comparison</a>
      </div>
    </div>
  </section>

  <section class="questions m-section">
    <div class="m-container">
      <span class="m-eyebrow">Five deciding questions</span>
      {#each [
        ['What should count toward the bill?', 'Piqae counts a job only when the node reports it complete. Jobs that remain failed, jammed, blocked, cancelled, or uncertain do not count; PrintNode says each API print request counts regardless of outcome.'],
        ['Do you need to inspect or modify the edge?', 'Piqae’s durable node and native boundaries are Apache-2.0, enabling audit and product-specific distributions subject to signing and trademark requirements.'],
        ['Who should operate queue durability?', 'Piqae Cloud operates storage, queue recovery, monitoring, backups, and upgrades. Self-hosters keep every core feature and own that operational work themselves.'],
        ['Are you serving many customer tenants?', 'Platform accounts and workspace boundaries let one integration manage customer printers and jobs without mixing operating contexts.'],
        ['Do you need an established client fleet today?', 'PrintNode is the more mature service and now offers a contact-sales Standalone Server. Keep it where that history or unsupported API behaviour outweighs Piqae’s cost and control benefits.']
      ] as item, index}
        <article>
          <span>{String(index + 1).padStart(2, '0')}</span>
          <h2>{item[0]}</h2>
          <p>{item[1]}</p>
        </article>
      {/each}
    </div>
  </section>

  <section class="decision m-section">
    <div class="m-container m-grid-2">
      <article class="m-card">
        <span>Evaluate Piqae when</span>
        <ul class="m-list">
          <li>You want the managed queue without a closed-source edge.</li>
          <li>Lower list pricing and reported-complete usage improve your economics.</li>
          <li>Your product needs tenant-scoped printers, profiles, jobs, keys, and webhooks.</li>
          <li>A credible self-hosted or custom-distribution path matters.</li>
        </ul>
      </article>
      <article class="m-card">
        <span>Stay with PrintNode when</span>
        <ul class="m-list">
          <li>You need generally available signed clients before Piqae passes its gates.</li>
          <li>Your workflow relies on PrintNode scales or unsupported API quirks.</li>
          <li>Its contact-sales Standalone Server already satisfies your deployment requirement.</li>
          <li>The migration effort is larger than the operating or commercial benefit.</li>
        </ul>
      </article>
    </div>
    <div class="m-container source">
      PrintNode product details were checked against
      <a href="https://www.printnode.com/en/features">official features</a> and
      <a href="https://www.printnode.com/en/pricing">pricing</a>, plus its
      <a href="https://www.printnode.com/en/faq">Standalone Server FAQ</a>, on 30 July 2026.
      Reported completion is Piqae’s strongest available OS, driver, or device signal; it does not
      prove ink reached paper when the hardware cannot report that evidence.
    </div>
  </section>

  <section class="m-section">
    <div class="m-narrow callout m-dark-panel">
      <h2>Start with one endpoint, then move the fleet.</h2>
      <p>
        The documented compatibility subset is designed to make the first change as small as a
        base URL and credential. Inventory your actual endpoints and driver dependencies, then use
        a virtual printer or explicitly authorised canary hardware before expanding.
      </p>
      <div class="m-actions">
        <a class="m-button primary" href="/migrate/printnode">Open the migration plan</a>
        <a class="m-button secondary" href="/docs/printnode-migration">Technical compatibility</a>
      </div>
    </div>
  </section>
</MarketingShell>

<style>
  .questions { background: #eeece6; }
  .questions article {
    display: grid; grid-template-columns: 70px 1fr 1fr; gap: 45px;
    padding: 35px 0; border-top: 1px solid var(--m-border);
  }
  .questions article > span { color: var(--m-violet-dark); font: 11px var(--font-mono); }
  .questions h2 { margin: 0; font-size: 24px; line-height: 1.15; letter-spacing: -.035em; }
  .questions p { margin: 0; color: var(--m-muted); }
  .decision article > span { color: var(--m-violet-dark); font-size: 11px; font-weight: 700; text-transform: uppercase; }
  .source { margin-top: 18px; color: var(--m-faint); font-size: 12px; }
  .source a { text-decoration: underline; }
  .callout { padding: clamp(30px, 6vw, 60px); }
  .callout h2 { max-width: 600px; margin: 0; font-size: 42px; letter-spacing: -.05em; }
  .callout p { max-width: 620px; }
  .secondary { border-color: var(--m-border-light); background: transparent; color: white; }
  @media (max-width: 700px) {
    .questions article { grid-template-columns: 1fr; gap: 12px; }
  }
</style>
