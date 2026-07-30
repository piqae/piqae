<script lang="ts">
  import { formatUsd } from '$lib/marketing/plans';
  import type { BillingInterval, PricingDisplay } from '$lib/marketing/types';

  let { plans }: { plans: PricingDisplay[] } = $props();

  let interval = $state<BillingInterval>('monthly');
  const wholeNumber = new Intl.NumberFormat('en-US');

  function priceFor(item: PricingDisplay): { amount: string; note: string } {
    if (interval === 'annual') {
      return {
        amount: formatUsd(Math.round(item.annualCents / 12)),
        note:
          item.annualCents === 0
            ? 'No card required'
            : `${formatUsd(item.annualCents)} billed annually`
      };
    }
    return {
      amount: formatUsd(item.monthlyCents),
      note: item.monthlyCents === 0 ? 'No card required' : 'Billed monthly'
    };
  }

  function acceptedJobs(item: PricingDisplay): { amount: number; period: string } {
    if (interval === 'annual' && item.annualIncludedAcceptedJobs !== null) {
      return { amount: item.annualIncludedAcceptedJobs, period: 'per year' };
    }
    return { amount: item.includedAcceptedJobs, period: 'per month' };
  }
</script>

<div class="billing-controls" role="group" aria-label="Billing interval">
  <button
    type="button"
    aria-pressed={interval === 'monthly'}
    class:active={interval === 'monthly'}
    onclick={() => (interval = 'monthly')}
  >
    Monthly
  </button>
  <button
    type="button"
    aria-pressed={interval === 'annual'}
    class:active={interval === 'annual'}
    onclick={() => (interval = 'annual')}
  >
    Annual <span>2 months free</span>
  </button>
</div>

<div class="plan-grid">
  {#each plans as item}
    <article class:featured={item.plan === 'pro'}>
      <header>
        <div class="plan-title">
          <h2>Piqae {item.name}</h2>
          {#if item.plan === 'pro'}<span class="badge">Best value</span>{/if}
        </div>
        <p>{item.headline}</p>
      </header>

      <div class="price">
        <div><strong>{priceFor(item).amount}</strong><span>/mo.</span></div>
        <small>{priceFor(item).note}</small>
      </div>

      <a
        class:primary={item.plan === 'pro'}
        class="plan-cta"
        href={`/start?plan=${item.plan}&interval=${interval}&source=pricing`}
      >
        {item.cta}
      </a>

      <div class="included">
        <h3>Included usage</h3>
        <dl>
          <div>
            <dt>Accepted jobs</dt>
            <dd>
              {wholeNumber.format(acceptedJobs(item).amount)}
              <small>{acceptedJobs(item).period}</small>
            </dd>
          </div>
          <div>
            <dt>Printer computers</dt>
            <dd>
              {wholeNumber.format(item.includedNodes)}
              <small>{item.includedNodes === 1 ? 'node' : 'nodes'}</small>
            </dd>
          </div>
        </dl>
      </div>

      <div class="features">
        <h3>What you get</h3>
        <ul>
          {#if item.plan === 'pro'}<li class="inherits">Everything in Piqae Free</li>{/if}
          {#each item.features as feature}<li>{feature}</li>{/each}
          <li>Unlimited workspace members</li>
          <li>Unlimited virtual test jobs</li>
        </ul>
      </div>
    </article>
  {/each}

  <article class="self-hosted">
    <header>
      <div class="plan-title"><h2>Self-hosted</h2></div>
      <p>Run the complete Apache-2.0 printing stack in your own environment.</p>
    </header>

    <div class="price">
      <div><strong>$0</strong></div>
      <small>Software licence</small>
    </div>

    <a class="plan-cta" href="/open-source">Explore self-hosting</a>

    <div class="included">
      <h3>Included usage</h3>
      <dl>
        <div>
          <dt>Accepted jobs</dt>
          <dd>Unlimited<small>self-hosted</small></dd>
        </div>
        <div>
          <dt>Printer computers</dt>
          <dd>Unlimited<small>your infrastructure</small></dd>
        </div>
      </dl>
    </div>

    <div class="features">
      <h3>What you get</h3>
      <ul>
        <li>Complete open-source stack</li>
        <li>Cloud, local, or private networking</li>
        <li>Community support</li>
        <li>No Piqae Cloud job charges</li>
        <li>Paid enterprise assistance available</li>
      </ul>
    </div>
  </article>
</div>

<style>
  .billing-controls {
    width: fit-content;
    display: flex;
    gap: 3px;
    margin: 0 auto 28px;
    padding: 4px;
    border: 1px solid rgb(255 255 255 / .18);
    border-radius: 12px;
    background: rgb(5 5 5 / .78);
    backdrop-filter: blur(14px);
  }
  .billing-controls button {
    min-height: 38px;
    padding: 0 14px;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: #aaa;
    cursor: pointer;
    font: 620 12px var(--m-font-body);
  }
  .billing-controls button.active {
    background: white;
    color: var(--m-ink);
  }
  .billing-controls span { color: #70aaff; }
  .plan-grid {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 18px;
  }
  article {
    min-width: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    padding: 30px;
    border: 1px solid rgb(23 22 27 / .13);
    border-radius: 13px;
    background: white;
    box-shadow: 0 25px 65px rgb(17 15 19 / .12);
  }
  article.featured { box-shadow: 0 28px 80px rgb(0 66 150 / .2); }
  header {
    min-height: 142px;
    padding-bottom: 26px;
    border-bottom: 1px solid var(--m-border);
  }
  .plan-title {
    min-height: 32px;
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  h2 {
    margin: 0;
    font-family: var(--m-font-display);
    font-size: 24px;
    font-weight: 650;
    letter-spacing: -.035em;
  }
  header p {
    margin: 10px 0 0;
    color: var(--m-muted);
    font-size: 14px;
    line-height: 1.5;
  }
  .badge {
    flex: none;
    padding: 6px 9px;
    border-radius: 5px;
    background: #f0f0ef;
    color: #313131;
    font: 650 10px/1 var(--font-mono);
    letter-spacing: .08em;
    text-transform: uppercase;
  }
  .price { padding: 33px 0 27px; }
  .price > div { min-height: 59px; display: flex; align-items: baseline; }
  .price strong {
    font-family: var(--m-font-display);
    font-size: clamp(48px, 5vw, 64px);
    font-weight: 680;
    letter-spacing: -.065em;
    line-height: .95;
  }
  .price span {
    margin-left: 3px;
    font-size: 25px;
    font-weight: 650;
    letter-spacing: -.04em;
  }
  .price small { display: block; margin-top: 8px; color: var(--m-faint); font-size: 12px; }
  .plan-cta {
    min-height: 46px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding-inline: 18px;
    border: 1.5px solid var(--m-violet);
    border-radius: 999px;
    color: var(--m-violet);
    font-size: 14px;
    font-weight: 680;
    transition:
      transform 180ms ease,
      background-color 180ms ease;
  }
  .plan-cta.primary { background: var(--m-violet); color: white; }
  .plan-cta:hover { transform: translateY(-1px); background: var(--m-violet-soft); }
  .plan-cta.primary:hover { background: var(--m-violet-dark); }
  .included,
  .features { margin-top: 38px; }
  .included h3,
  .features h3 {
    margin: 0 0 12px;
    font-family: var(--m-font-display);
    font-size: 13px;
    font-weight: 700;
    letter-spacing: -.01em;
  }
  dl { margin: 0; }
  dl > div {
    min-height: 54px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    border-top: 1px solid var(--m-border);
  }
  dt { color: var(--m-muted); font-size: 12px; }
  dd { margin: 0; font-size: 13px; font-weight: 680; text-align: right; }
  dd small { display: block; color: var(--m-faint); font-size: 9px; font-weight: 500; }
  ul { display: grid; gap: 8px; margin: 0; padding: 0; list-style: none; }
  li {
    position: relative;
    min-height: 42px;
    display: flex;
    align-items: center;
    padding: 10px 12px 10px 39px;
    border-radius: 8px;
    background: #f5f5f4;
    color: var(--m-ink);
    font-size: 12px;
    line-height: 1.35;
  }
  li::before {
    position: absolute;
    left: 15px;
    width: 9px;
    height: 5px;
    border-left: 1.5px solid var(--m-ink);
    border-bottom: 1.5px solid var(--m-ink);
    content: '';
    transform: translateY(-1px) rotate(-45deg);
  }
  li.inherits { background: white; font-weight: 650; }
  @media (max-width: 980px) {
    .plan-grid { grid-template-columns: 1fr 1fr; }
    .self-hosted { grid-column: 1 / -1; }
    .self-hosted header { min-height: 115px; }
  }
  @media (max-width: 680px) {
    .billing-controls { margin-bottom: 18px; }
    .billing-controls button { padding-inline: 10px; }
    .plan-grid { grid-template-columns: 1fr; }
    .self-hosted { grid-column: auto; }
    article { padding: 25px 22px; border-radius: 11px; }
    header,
    .self-hosted header { min-height: 0; }
    .price strong { font-size: 54px; }
  }
</style>
