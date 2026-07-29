<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import PricingCards from '$lib/components/marketing/PricingCards.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import { formatUsd } from '$lib/marketing/plans';
  import type { CloudPricingCatalog } from '$lib/marketing/types';

  let { data }: { data: { pricing: CloudPricingCatalog } } = $props();

  let pricingStructuredData = $derived([
    {
      '@context': 'https://schema.org',
      '@type': 'Product',
      name: 'Piqae Cloud',
      category: 'Printing API',
      offers: data.pricing.plans.map((plan) => ({
        '@type': 'Offer',
        name: `Piqae Cloud ${plan.name}`,
        priceCurrency: 'USD',
        price: (plan.monthlyCents / 100).toFixed(2),
        availability: 'https://schema.org/PreOrder',
        url: `/start?plan=${plan.plan}&interval=monthly`
      }))
    },
    {
      '@context': 'https://schema.org',
      '@type': 'FAQPage',
      mainEntity: [
        {
          '@type': 'Question',
          name: 'Does spooler acceptance mean the page printed?',
          acceptedAnswer: {
            '@type': 'Answer',
            text: 'No. It means the operating system accepted the handoff. Hardware may still fail, jam, or report an ambiguous result.'
          }
        },
        {
          '@type': 'Question',
          name: 'What happens when the Free plan reaches its limit?',
          acceptedAnswer: {
            '@type': 'Answer',
            text: 'New Cloud jobs are rejected with quota_exceeded until the workspace upgrades. Already accepted jobs continue delivery.'
          }
        },
        {
          '@type': 'Question',
          name: 'Are test jobs billable?',
          acceptedAnswer: {
            '@type': 'Answer',
            text: 'No. Only live-environment jobs that reach the OS-spooler-accepted event enter the usage ledger.'
          }
        },
        {
          '@type': 'Question',
          name: 'Can pricing shown here differ from checkout?',
          acceptedAnswer: {
            '@type': 'Answer',
            text: 'The server-owned Piqae catalog controls plan amounts and limits. Checkout verifies the matching Stripe price before showing tax and the final charge.'
          }
        }
      ]
    }
  ]);
</script>

<Seo
  title="Piqae pricing — Cloud and self-hosted printing"
  description="Start free, pay for jobs accepted by the operating-system spooler, or run the Apache-2.0 Piqae stack on your own infrastructure."
  path="/pricing"
  structuredData={pricingStructuredData}
/>

<MarketingShell announcement="Pricing is preview-only until the infrastructure margin and release gates pass">
  <section class="m-page-hero">
    <div class="m-container centered">
      <span class="m-eyebrow">Pricing</span>
      <h1 class="m-title">Start free. Scale without surprises.</h1>
      <p class="m-lede">
        Choose managed Piqae Cloud or run the open-source stack yourself. Cloud plans include
        unlimited workspace members and count each job only when the local print system accepts it.
      </p>
    </div>
  </section>

  <section class="m-container">
    <PricingCards plans={data.pricing.plans} />
    <p class="pricing-disclaimer">
      Prices are USD excluding tax and come from server catalog {data.pricing.version}. Checkout
      must match the same amount in Stripe before it can open. Sales remain disabled until launch
      gates pass.
    </p>
  </section>

  <section class="m-section">
    <div class="m-container">
      <div class="self-hosted m-dark-panel">
        <div>
          <span>Apache-2.0</span>
          <h2>Self-hosted is $0.</h2>
          <p>
            Run the control plane, database, object storage, and agents in your environment.
            Unlimited self-hosted jobs; your team owns the infrastructure and operations.
          </p>
        </div>
        <ul>
          <li>Complete open-source stack</li>
          <li>Community support</li>
          <li>Cloud job charges do not apply</li>
          <li>Paid enterprise assistance available</li>
        </ul>
        <a class="m-button primary" href="/open-source">Explore self-hosting</a>
      </div>
    </div>
  </section>

  <section class="definitions m-section">
    <div class="m-container">
      <span class="m-eyebrow">What counts</span>
      <h2 class="m-heading">Only accepted jobs count.</h2>
      <div class="m-grid-3">
        <article class="m-card">
          <h3>Accepted job</h3>
          <p>
            Counted once after a live-environment agent reports that the OS spooler accepted the
            job. Test jobs and pre-handoff failures are free.
          </p>
        </article>
        <article class="m-card">
          <h3>Node allowance</h3>
          <p>
            A node is one enrolled computer running Piqae. Workspace members and printers do not
            consume the node allowance.
          </p>
        </article>
        <article class="m-card">
          <h3>Retention</h3>
          <p>
            Job metadata and document content have separate private-beta policy targets. Automated
            deletion enforcement remains a release gate and is not represented as active yet.
          </p>
        </article>
      </div>
    </div>
  </section>

  <section class="m-section">
    <div class="m-container">
      <span class="m-eyebrow">Overages and limits</span>
      <div class="m-table-wrap">
        <table class="m-table">
          <thead>
            <tr><th>Plan</th><th>Job overage</th><th>Node policy</th><th>Quota behavior</th></tr>
          </thead>
          <tbody>
            {#each data.pricing.plans as plan}
              <tr>
                <td>{plan.name}</td>
                <td>
                  {plan.jobOverageCents === null || plan.jobOverageUnit === null
                    ? 'No automatic overage'
                    : `${formatUsd(plan.jobOverageCents)} / ${plan.jobOverageUnit.toLocaleString('en-US')}`}
                </td>
                <td>{plan.includedNodes} included; upgrade or contact us to add more</td>
                <td>
                  {plan.quotaBehavior === 'blocked'
                    ? 'New jobs rejected at the limit'
                    : 'Printing continues and overage applies'}
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    </div>
  </section>

  <section class="faq m-section">
    <div class="m-narrow">
      <span class="m-eyebrow">Questions</span>
      <details>
        <summary>Does spooler acceptance mean the page printed?</summary>
        <p>No. It means the operating system accepted the handoff. Hardware may still fail, jam, or report an ambiguous result.</p>
      </details>
      <details>
        <summary>What happens when the Free plan reaches its limit?</summary>
        <p>New Cloud jobs are rejected with a structured <code>quota_exceeded</code> response. Jobs already accepted continue delivery.</p>
      </details>
      <details>
        <summary>Are test jobs billable?</summary>
        <p>No. Only live-environment jobs that reach the OS-spooler-accepted event enter the usage ledger.</p>
      </details>
      <details>
        <summary>How does the annual Pro allowance work?</summary>
        <p>
          Monthly Pro includes 25,000 accepted jobs each month. Annual Pro includes 300,000
          accepted jobs across its annual Stripe billing period, with overage measured against
          that annual allowance.
        </p>
      </details>
      <details>
        <summary>Can pricing shown here differ from checkout?</summary>
        <p>No plan amount or limit is editable in the CMS. Checkout opens only after the configured Stripe price matches the server catalog; Stripe then confirms tax and the final charge.</p>
      </details>
    </div>
  </section>
</MarketingShell>

<style>
  .centered { display: grid; justify-items: center; text-align: center; }
  .centered .m-lede { margin-inline: auto; }
  .pricing-disclaimer { margin: 22px 0 0; color: var(--m-faint); text-align: center; font-size: 11px; }
  .self-hosted {
    display: grid;
    grid-template-columns: 1.3fr 1fr auto;
    align-items: center;
    gap: 55px;
    padding: clamp(30px, 5vw, 58px);
  }
  .self-hosted span { color: #71adff; font-size: 11px; font-weight: 700; text-transform: uppercase; }
  .self-hosted h2 { margin: 10px 0; font-size: 42px; letter-spacing: -.05em; }
  .self-hosted p { max-width: 570px; margin: 0; }
  .self-hosted ul { display: grid; gap: 8px; padding: 0; margin: 0; color: #aaa8b1; list-style: none; font-size: 12px; }
  .self-hosted li::before { margin-right: 8px; color: var(--m-green); content: '✓'; }
  .definitions { background: #eeece6; }
  .definitions .m-heading { margin-bottom: 40px; }
  .faq { background: #eeece6; }
  details { border-top: 1px solid var(--m-border); }
  details:last-child { border-bottom: 1px solid var(--m-border); }
  summary { padding: 22px 0; color: var(--m-ink); font-weight: 620; cursor: pointer; }
  details p { margin: -5px 0 24px; color: var(--m-muted); }
  @media (max-width: 900px) {
    .self-hosted { grid-template-columns: 1fr; gap: 28px; }
  }
</style>
