<script lang="ts">
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import PricingPagePlans from '$lib/components/marketing/PricingPagePlans.svelte';
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
          name: 'What is a reported-complete job?',
          acceptedAnswer: {
            '@type': 'Answer',
            text: 'It is a Live job for which the node or operating system reports completion. It is the strongest available completion signal, but it is not physical proof that ink reached paper.'
          }
        },
        {
          '@type': 'Question',
          name: 'What happens when the Free plan reaches its limit?',
          acceptedAnswer: {
            '@type': 'Answer',
            text: 'New Cloud jobs are rejected with quota_exceeded until the workspace upgrades. Jobs already registered continue their durable lifecycle.'
          }
        },
        {
          '@type': 'Question',
          name: 'Are test jobs billable?',
          acceptedAnswer: {
            '@type': 'Answer',
            text: 'No. Only Live-environment jobs reported complete enter the usage ledger. Failed, blocked or jammed, cancelled, expired, and delivery-uncertain jobs do not count.'
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
  description="Start free and pay only for Cloud jobs reported complete, choose Enterprise, or run the complete Apache-2.0 Piqae stack yourself."
  path="/pricing"
  structuredData={pricingStructuredData}
/>

<MarketingShell>
  <section class="pricing-hero">
    <div class="print-texture" aria-hidden="true">
      <span class="label label-one"></span>
      <span class="label label-two"></span>
      <span class="label label-three"></span>
      <span class="feed feed-one"></span>
      <span class="feed feed-two"></span>
    </div>
    <div class="m-container hero-copy">
      <span class="m-eyebrow">Pricing</span>
      <h1>Pay only for prints reported complete.</h1>
      <p class="m-lede">
        Failed, jammed or blocked, cancelled, expired, and delivery-uncertain jobs do not consume
        Cloud usage. Start free, scale with Pro or Enterprise, or operate the open-source stack.
      </p>
    </div>
  </section>

  <section class="plans-stage">
    <div class="m-container">
      <PricingPagePlans plans={data.pricing.plans} />
    </div>
    <p class="pricing-disclaimer">
      Prices are USD excluding tax. Final charges are confirmed at checkout. Cloud sales remain in
      preview until the infrastructure margin and release gates pass. Catalog {data.pricing.version}.
    </p>
  </section>

  <section class="definitions m-section">
    <div class="m-container">
      <div class="section-intro">
        <span class="m-eyebrow">Straightforward usage</span>
        <h2 class="m-heading">Pay for printing, not failed attempts.</h2>
        <p>
          Piqae counts a Cloud job only after the node or operating system reports completion.
          Test jobs and any job that ends failed, blocked, cancelled, expired, or uncertain do
          not consume paid usage.
        </p>
      </div>
      <div class="m-grid-3">
        <article class="m-card">
          <span class="definition-number">01</span>
          <h3>Reported-complete job</h3>
          <p>
            Counted once when a Live job reaches <code>completed_reported</code>. This is the
            strongest available system signal, not a claim that Piqae physically inspected the page.
          </p>
        </article>
        <article class="m-card">
          <span class="definition-number">02</span>
          <h3>Node allowance</h3>
          <p>
            A node is one enrolled computer running Piqae. Workspace members and printers do not
            consume the node allowance.
          </p>
        </article>
        <article class="m-card">
          <span class="definition-number">03</span>
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
      <div class="table-intro">
        <div>
          <span class="m-eyebrow">Compare Cloud plans</span>
          <h2 class="m-heading">The details, side by side.</h2>
        </div>
        <p>Human dashboard users and virtual test jobs are unlimited on every Cloud plan.</p>
      </div>
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
      <h2 class="m-heading">Good to know.</h2>
      <details>
        <summary>What is a reported-complete job?</summary>
        <p>A Live job for which the node or operating system reports completion. It is the strongest available signal, but it is not physical proof that ink reached paper.</p>
      </details>
      <details>
        <summary>What happens when the Free plan reaches its limit?</summary>
        <p>New Cloud jobs are rejected with a structured <code>quota_exceeded</code> response. Jobs already registered continue their durable lifecycle.</p>
      </details>
      <details>
        <summary>Are test jobs billable?</summary>
        <p>No. Only Live-environment jobs reported complete enter the usage ledger. Failed, blocked or jammed, cancelled, expired, and delivery-uncertain jobs do not count.</p>
      </details>
      <details>
        <summary>How does the annual Pro allowance work?</summary>
        <p>
          Monthly Pro includes 25,000 reported-complete jobs each month. Annual Pro includes
          300,000 reported-complete jobs across its annual Stripe billing period, with overage
          measured against that annual allowance.
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
  .pricing-hero {
    position: relative;
    min-height: 650px;
    overflow: hidden;
    background:
      radial-gradient(circle at 20% 40%, rgb(0 106 255 / .3), transparent 32%),
      radial-gradient(circle at 82% 18%, rgb(70 154 255 / .22), transparent 26%),
      linear-gradient(135deg, #101216 0%, #07121f 52%, #111 100%);
    color: white;
  }
  .pricing-hero::after {
    position: absolute;
    inset: 0;
    background:
      linear-gradient(180deg, transparent 55%, rgb(0 0 0 / .42)),
      repeating-radial-gradient(circle at 20% 30%, rgb(255 255 255 / .025) 0 1px, transparent 1px 4px);
    content: '';
    mix-blend-mode: screen;
    pointer-events: none;
  }
  .hero-copy {
    position: relative;
    z-index: 2;
    display: grid;
    justify-items: center;
    padding-top: clamp(76px, 9vw, 120px);
    text-align: center;
  }
  .hero-copy .m-eyebrow { color: white; font: 620 12px/1 var(--font-mono); letter-spacing: .08em; text-transform: uppercase; }
  .hero-copy h1 {
    max-width: 980px;
    margin: 20px 0 0;
    color: white;
    font-family: var(--m-font-editorial);
    font-size: clamp(52px, 7vw, 92px);
    font-weight: 400;
    letter-spacing: -.055em;
    line-height: .94;
    text-wrap: balance;
  }
  .hero-copy .m-lede {
    max-width: 680px;
    margin-top: 28px;
    color: rgb(255 255 255 / .76);
    font-size: 17px;
  }
  .print-texture { position: absolute; inset: 0; overflow: hidden; filter: blur(.15px); }
  .label,
  .feed {
    position: absolute;
    display: block;
    border: 1px solid rgb(255 255 255 / .12);
    background:
      repeating-linear-gradient(180deg, transparent 0 15px, rgb(255 255 255 / .07) 15px 16px),
      rgb(255 255 255 / .035);
    box-shadow: 0 30px 70px rgb(0 0 0 / .32);
    transform: rotate(var(--rotation));
  }
  .label { width: 190px; height: 250px; border-radius: 12px; }
  .label::before {
    position: absolute;
    top: 28px;
    left: 28px;
    width: 72px;
    height: 72px;
    border: 14px solid rgb(255 255 255 / .09);
    content: '';
  }
  .label-one { --rotation: -18deg; top: 85px; left: -40px; }
  .label-two { --rotation: 16deg; top: -90px; right: 8%; opacity: .7; }
  .label-three { --rotation: 8deg; right: -50px; bottom: -70px; }
  .feed { width: 65px; height: 520px; border-radius: 6px; }
  .feed-one { --rotation: 38deg; top: -180px; left: 27%; opacity: .45; }
  .feed-two { --rotation: -42deg; right: 26%; bottom: -260px; opacity: .35; }
  .plans-stage {
    position: relative;
    z-index: 3;
    margin-top: -175px;
    padding-bottom: clamp(82px, 10vw, 130px);
  }
  .pricing-disclaimer {
    width: min(900px, calc(100% - 48px));
    margin: 24px auto 0;
    color: var(--m-faint);
    text-align: center;
    font-size: 10px;
  }
  .definitions { background: #eeece6; }
  .section-intro { max-width: 820px; margin-bottom: 54px; }
  .section-intro > p { max-width: 680px; margin: 24px 0 0; color: var(--m-muted); font-size: 17px; }
  .definitions .m-card { min-height: 310px; display: flex; flex-direction: column; border-radius: 12px; background: #f8f8f6; }
  .definitions .m-card h3 { margin-top: auto; font-size: 23px; }
  .definitions .m-card p { font-size: 14px; }
  .definition-number { color: var(--m-violet-dark); font: 11px var(--font-mono); }
  .table-intro {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 40px;
    margin-bottom: 50px;
  }
  .table-intro > p { max-width: 360px; margin: 0 0 5px; color: var(--m-muted); }
  .m-table-wrap { border-radius: 13px; background: white; }
  .faq { background: #eeece6; }
  .faq .m-heading { margin-bottom: 42px; }
  details { border-top: 1px solid var(--m-border); }
  details:last-child { border-bottom: 1px solid var(--m-border); }
  summary { padding: 25px 0; color: var(--m-ink); font-size: 17px; font-weight: 620; cursor: pointer; }
  details p { margin: -5px 0 24px; color: var(--m-muted); }
  @media (max-width: 900px) {
    .pricing-hero { min-height: 610px; }
    .plans-stage { margin-top: -150px; }
    .table-intro { display: grid; }
  }
  @media (max-width: 680px) {
    .pricing-hero { min-height: 580px; }
    .hero-copy { padding-top: 70px; }
    .hero-copy h1 { font-size: 50px; }
    .hero-copy .m-lede { font-size: 15px; }
    .plans-stage { margin-top: -120px; }
    .label { opacity: .45; }
    .definitions .m-card { min-height: 245px; }
  }
</style>
