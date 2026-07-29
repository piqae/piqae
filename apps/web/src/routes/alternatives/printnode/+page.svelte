<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import { printNodePricingReviewDueAt } from '$lib/marketing/calculator';

  const claimsExpired = new Date() > new Date(`${printNodePricingReviewDueAt}T23:59:59Z`);
</script>

<Seo
  title="A PrintNode alternative for open and self-hosted printing"
  description="A decision guide for teams seeking a PrintNode alternative with open source, self-hosting, durable edge queues, or multi-tenant control."
  path="/alternatives/printnode"
  noindex={claimsExpired}
/>

<MarketingShell announcement={claimsExpired ? 'Comparison evidence is past its review date and excluded from search' : undefined}>
  <section class="m-page-hero">
    <div class="m-container">
      <span class="m-eyebrow">PrintNode alternative</span>
      <h1 class="m-title">An alternative is useful only when it changes your constraints.</h1>
      <p class="m-lede">
        Piqae is not a reason to migrate a working print system by itself. It becomes relevant when
        open source, deployment control, queue ownership, or multi-tenant economics materially
        change the decision.
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
        ['Must the control plane run in your environment?', 'If yes, a hosted-only operating model is a structural mismatch. Piqae keeps self-hosting first-class.'],
        ['Do you need to inspect or modify the edge software?', 'Piqae’s durable agent and native boundaries are available under Apache-2.0.'],
        ['Does ambiguous delivery need an explicit workflow?', 'Piqae preserves uncertainty instead of treating a native handoff as physical proof.'],
        ['Are you operating many customer tenants?', 'Pro includes platform customer accounts, while workspace boundaries preserve each customer’s operating context.'],
        ['Is the migration risk lower than the expected benefit?', 'If not, keep the current system. A controlled canary and tested rollback are requirements, not optional polish.']
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
          <li>A credible self-hosted exit is a procurement requirement.</li>
          <li>Your application needs PDF and RAW through one durable job model.</li>
          <li>You can canary the currently tested compatibility subset.</li>
          <li>You are comfortable validating preview platform support before launch.</li>
        </ul>
      </article>
      <article class="m-card">
        <span>Stay with PrintNode when</span>
        <ul class="m-list">
          <li>You need generally available signed clients before Piqae passes its gates.</li>
          <li>Your workflow relies on PrintNode scales or unsupported API quirks.</li>
          <li>Self-hosting and source access do not create meaningful value.</li>
          <li>The migration effort is larger than the operating or commercial benefit.</li>
        </ul>
      </article>
    </div>
    <div class="m-container source">
      PrintNode product details were checked against
      <a href="https://www.printnode.com/en/features">official features</a> and
      <a href="https://www.printnode.com/en/pricing">pricing</a> on 29 July 2026.
    </div>
  </section>

  <section class="m-section">
    <div class="m-narrow callout m-dark-panel">
      <h2>Prove one real workflow before planning the switch.</h2>
      <p>
        Inventory endpoints, drivers, formats, options, and failure handling. Then use a virtual
        printer or explicitly authorised canary hardware inside the tested support envelope.
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
