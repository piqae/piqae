<script lang="ts">
  import { formatUsd } from '$lib/marketing/plans';
  import type { PricingDisplay } from '$lib/marketing/types';
  import type { BillingInterval } from '$lib/marketing/types';

  let {
    condensed = false,
    homepage = false,
    plans
  }: {
    condensed?: boolean;
    homepage?: boolean;
    plans: PricingDisplay[];
  } = $props();
  let interval = $state<BillingInterval>('monthly');
  const wholeNumber = new Intl.NumberFormat('en-US');

  function priceFor(item: PricingDisplay): { amount: string; note?: string } {
    if (interval === 'annual') {
      return {
        amount: formatUsd(Math.round(item.annualCents / 12)),
        note: item.annualCents > 0
          ? `${formatUsd(item.annualCents)} billed annually`
          : undefined
      };
    }

    return {
      amount: formatUsd(item.monthlyCents)
    };
  }

  function overageFor(item: PricingDisplay): string {
    if (item.jobOverageCents === null || item.jobOverageUnit === null) {
      return 'Upgrade required';
    }
    return `${formatUsd(item.jobOverageCents)} / ${wholeNumber.format(item.jobOverageUnit)}`;
  }

  function acceptedJobsFor(item: PricingDisplay): { amount: number; period: string } {
    if (interval === 'annual' && item.annualIncludedAcceptedJobs !== null) {
      return { amount: item.annualIncludedAcceptedJobs, period: 'per annual billing period' };
    }
    return { amount: item.includedAcceptedJobs, period: 'per month' };
  }

  function retentionMaximum(hours: number): string {
    return hours % 24 === 0 ? `${hours / 24} days` : `${wholeNumber.format(hours)} hours`;
  }
</script>

<div id="pro" class="pricing-anchor" aria-hidden="true"></div>

{#if homepage}
  <div class="pricing-controls home" role="group" aria-label="Billing interval">
    <button
      type="button"
      aria-pressed={interval === 'monthly'}
      class:active={interval === 'monthly'}
      onclick={() => (interval = 'monthly')}
    >Monthly</button>
    <button
      type="button"
      aria-pressed={interval === 'annual'}
      class:active={interval === 'annual'}
      onclick={() => (interval = 'annual')}
    >
      Annual <span>2 months free</span>
    </button>
  </div>

  <div class="home-pricing-grid">
    {#each plans as item}
      <article class="home-plan" class:featured={item.plan === 'pro'}>
        <div class="home-plan-header">
          <div class="home-plan-title">
            <h3>Piqae {item.name}</h3>
            {#if item.badge}<span class="badge">{item.badge}</span>{/if}
          </div>
          <p>{item.headline}</p>
        </div>
        <div class="home-plan-cost">
          <div>
            <strong>{priceFor(item).amount}</strong><span>/mo.</span>
          </div>
          {#if priceFor(item).note}
            <small>{priceFor(item).note}</small>
          {:else}
            <small aria-hidden="true">&nbsp;</small>
          {/if}
          <p>
            {wholeNumber.format(acceptedJobsFor(item).amount)} accepted jobs
            <span>
              {acceptedJobsFor(item).period === 'per annual billing period' ? 'per year' : 'per month'}
              · {wholeNumber.format(item.includedNodes)} {item.includedNodes === 1 ? 'node' : 'nodes'}
            </span>
          </p>
        </div>
        <a
          class="home-plan-cta"
          class:primary={item.plan === 'pro'}
          href={`/start?plan=${item.plan}&interval=${interval}&source=home-pricing`}
        >
          {item.cta}
        </a>
      </article>
    {/each}
  </div>
{:else}
  <div class="pricing-controls" role="group" aria-label="Billing interval">
    <button
      type="button"
      aria-pressed={interval === 'monthly'}
      class:active={interval === 'monthly'}
      onclick={() => (interval = 'monthly')}
    >Monthly</button>
    <button
      type="button"
      aria-pressed={interval === 'annual'}
      class:active={interval === 'annual'}
      onclick={() => (interval = 'annual')}
    >
      Annual <span>2 months free</span>
    </button>
  </div>

{#if condensed}
  <div class="pricing-grid">
    {#each plans as item}
      <article class:featured={item.plan === 'pro'}>
        <div class="plan-top">
          <div>
            <h3>{item.name}</h3>
            <p>{item.headline}</p>
          </div>
          {#if item.badge}<span class="badge">{item.badge}</span>{/if}
        </div>
        <div class="price">
          <strong>{priceFor(item).amount}</strong><span>/ month</span>
          {#if priceFor(item).note}<small>{priceFor(item).note}</small>{/if}
        </div>
        <ul>
          {#each item.features as feature}<li>{feature}</li>{/each}
        </ul>
        <a
          class="m-button"
          class:primary={item.plan === 'pro'}
          href={`/start?plan=${item.plan}&interval=${interval}&source=pricing`}
        >
          {item.cta}
        </a>
      </article>
    {/each}
  </div>
{:else}
  <div class="plan-table-wrap">
    <table class="plan-table">
      <thead>
        <tr>
          <th scope="col" class="row-label">
            <span>Piqae Cloud</span>
            <strong>Compare plans</strong>
          </th>
          {#each plans as item}
            <th scope="col" class:featured={item.plan === 'pro'}>
              <div class="plan-name">
                <strong>{item.name}</strong>
                {#if item.badge}<span class="badge">{item.badge}</span>{/if}
              </div>
              <p>{item.headline}</p>
              <div class="price">
                <strong>{priceFor(item).amount}</strong><span>/ month</span>
                {#if priceFor(item).note}<small>{priceFor(item).note}</small>{/if}
              </div>
            </th>
          {/each}
        </tr>
      </thead>
      <tbody>
        <tr>
          <th scope="row">Accepted jobs</th>
          {#each plans as item}
            <td class:featured={item.plan === 'pro'}>
              <strong>{wholeNumber.format(acceptedJobsFor(item).amount)}</strong>
              <small>{acceptedJobsFor(item).period}</small>
            </td>
          {/each}
        </tr>
        <tr>
          <th scope="row">Nodes</th>
          {#each plans as item}
            <td class:featured={item.plan === 'pro'}>
              <strong>{wholeNumber.format(item.includedNodes)}</strong>
              <small>included</small>
            </td>
          {/each}
        </tr>
        <tr>
          <th scope="row">Customer accounts</th>
          {#each plans as item}
            <td class:featured={item.plan === 'pro'}>
              <strong>{item.customerAccounts === 'included' ? 'Included' : '—'}</strong>
              <small>{item.customerAccounts === 'included' ? 'not separately metered' : 'Pro only'}</small>
            </td>
          {/each}
        </tr>
        <tr>
          <th scope="row">Metadata retention policy</th>
          {#each plans as item}
            <td class:featured={item.plan === 'pro'}>
              <strong>{item.metadataRetentionDays} days</strong><small>preview target</small>
            </td>
          {/each}
        </tr>
        <tr>
          <th scope="row">Document retention policy</th>
          {#each plans as item}
            <td class:featured={item.plan === 'pro'}>
              <strong>
                {item.documentRetention.configurable
                  ? `Up to ${retentionMaximum(item.documentRetention.maximumHours)}`
                  : `${item.documentRetention.defaultHours} hours`}
              </strong>
              <small>preview target</small>
            </td>
          {/each}
        </tr>
        <tr>
          <th scope="row">Additional jobs</th>
          {#each plans as item}
            <td class:featured={item.plan === 'pro'}><strong>{overageFor(item)}</strong></td>
          {/each}
        </tr>
        <tr class="action-row">
          <th scope="row">Unlimited workspace members on every plan</th>
          {#each plans as item}
            <td class:featured={item.plan === 'pro'}>
              <a
                class="m-button"
                class:primary={item.plan === 'pro'}
                href={`/start?plan=${item.plan}&interval=${interval}&source=pricing`}
              >
                {item.cta}
              </a>
            </td>
          {/each}
        </tr>
      </tbody>
    </table>
  </div>

  <div class="mobile-plans">
    {#each plans as item}
      <article class:featured={item.plan === 'pro'}>
        <div class="plan-top">
          <div><h3>{item.name}</h3><p>{item.headline}</p></div>
          {#if item.badge}<span class="badge">{item.badge}</span>{/if}
        </div>
        <div class="price">
          <strong>{priceFor(item).amount}</strong><span>/ month</span>
          {#if priceFor(item).note}<small>{priceFor(item).note}</small>{/if}
        </div>
        <dl>
          <div>
            <dt>Accepted jobs</dt>
            <dd>
              {wholeNumber.format(acceptedJobsFor(item).amount)}
              · {acceptedJobsFor(item).period}
            </dd>
          </div>
          <div><dt>Nodes</dt><dd>{wholeNumber.format(item.includedNodes)}</dd></div>
          <div><dt>Metadata policy</dt><dd>{item.metadataRetentionDays} days · preview</dd></div>
          <div><dt>Additional jobs</dt><dd>{overageFor(item)}</dd></div>
        </dl>
        <a
          class="m-button"
          class:primary={item.plan === 'pro'}
          href={`/start?plan=${item.plan}&interval=${interval}&source=pricing`}
        >
          {item.cta}
        </a>
      </article>
    {/each}
  </div>
{/if}
{/if}

<style>
  .pricing-anchor { scroll-margin-top: 30px; }
  .pricing-controls {
    width: fit-content;
    display: flex;
    gap: 3px;
    margin: 32px auto 22px;
    padding: 4px;
    border: 1px solid var(--m-border);
    border-radius: 11px;
    background: rgb(255 255 255 / 0.58);
  }
  .pricing-controls button {
    min-height: 36px;
    padding: 0 12px;
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--m-muted);
    cursor: pointer;
    font-size: 12px;
    font-weight: 620;
  }
  .pricing-controls button.active {
    background: white;
    color: var(--m-ink);
    box-shadow: 0 1px 5px rgb(23 22 27 / 0.08);
  }
  .pricing-controls span { color: var(--m-violet-dark); }
  .pricing-grid,
  .mobile-plans {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
    max-width: 820px;
    margin-inline: auto;
  }
  article {
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding: 24px 20px;
    border: 1px solid var(--m-border);
    border-radius: 16px;
    background: rgb(255 255 255 / 0.62);
  }
  article.featured {
    border-color: rgb(0 106 255 / 0.45);
    background: white;
    box-shadow: 0 15px 50px rgb(0 70 160 / 0.09);
  }
  .plan-top { min-height: 86px; }
  h3 { margin: 0; font-size: 18px; letter-spacing: -0.03em; }
  .plan-top p {
    margin: 7px 0 0;
    color: var(--m-muted);
    font-size: 12px;
    line-height: 1.45;
  }
  .badge {
    display: inline-flex;
    margin-top: 9px;
    padding: 3px 7px;
    border-radius: 99px;
    background: var(--m-violet-soft);
    color: var(--m-violet-dark);
    font-size: 10px;
    font-weight: 700;
  }
  .price { min-height: 86px; padding-top: 20px; }
  .price strong { font-size: 30px; font-weight: 620; letter-spacing: -0.05em; }
  .price > span { color: var(--m-faint); font-size: 11px; }
  .price small { display: block; color: var(--m-faint); font-size: 10px; }
  ul {
    flex: 1;
    display: grid;
    align-content: start;
    gap: 9px;
    padding: 18px 0 24px;
    margin: 0;
    border-top: 1px solid var(--m-border);
    list-style: none;
  }
  li {
    position: relative;
    padding-left: 16px;
    color: var(--m-muted);
    font-size: 11px;
    line-height: 1.45;
  }
  li::before {
    position: absolute;
    left: 0;
    top: 0.55em;
    width: 6px;
    height: 4px;
    border-left: 1.5px solid var(--m-green);
    border-bottom: 1.5px solid var(--m-green);
    content: '';
    transform: rotate(-45deg);
  }
  article .m-button { width: 100%; min-height: 40px; font-size: 12px; }
  .plan-table-wrap {
    overflow-x: auto;
    border: 1px solid var(--m-border);
    border-radius: 20px;
    background: white;
  }
  .plan-table {
    width: 100%;
    min-width: 680px;
    border-collapse: collapse;
    table-layout: fixed;
  }
  .plan-table th,
  .plan-table td {
    padding: 24px 18px;
    border-right: 1px solid var(--m-border);
    border-bottom: 1px solid var(--m-border);
    text-align: left;
    vertical-align: top;
  }
  .plan-table tr > :last-child { border-right: 0; }
  .plan-table tbody tr:last-child > * { border-bottom: 0; }
  .plan-table .row-label,
  .plan-table tbody th { width: 180px; background: #f6f6f4; }
  .plan-table .row-label span {
    display: block;
    color: var(--m-violet-dark);
    font-size: 11px;
    font-weight: 720;
    letter-spacing: .07em;
    text-transform: uppercase;
  }
  .plan-table .row-label strong {
    display: block;
    margin-top: 34px;
    font-size: 21px;
    letter-spacing: -.035em;
  }
  .plan-table thead th { height: 275px; }
  .plan-table thead th.featured,
  .plan-table tbody td.featured { background: #f0f6ff; }
  .plan-name {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .plan-name > strong { font-size: 20px; letter-spacing: -.035em; }
  .plan-table thead p {
    min-height: 48px;
    margin: 8px 0 0;
    color: var(--m-muted);
    font-size: 12px;
    font-weight: 450;
  }
  .plan-table .price { padding-top: 26px; }
  .plan-table tbody th {
    color: var(--m-muted);
    font-size: 12px;
    font-weight: 620;
  }
  .plan-table tbody td > strong {
    display: block;
    color: var(--m-ink);
    font-size: 13px;
    font-weight: 650;
  }
  .plan-table tbody td > small {
    display: block;
    margin-top: 2px;
    color: var(--m-faint);
    font-size: 10px;
  }
  .plan-table .action-row td { padding-block: 18px; vertical-align: middle; }
  .plan-table .action-row .m-button { width: 100%; min-height: 40px; font-size: 12px; }
  .mobile-plans { display: none; }
  .mobile-plans dl {
    display: grid;
    gap: 10px;
    padding: 18px 0 24px;
    margin: 0;
    border-top: 1px solid var(--m-border);
  }
  .mobile-plans dl div { display: flex; justify-content: space-between; gap: 18px; }
  .mobile-plans dt { color: var(--m-muted); font-size: 11px; }
  .mobile-plans dd { margin: 0; color: var(--m-ink); font-size: 11px; font-weight: 650; text-align: right; }
  .pricing-controls.home {
    margin: 68px 0 34px auto;
    border-color: rgb(255 255 255 / .18);
    background: #171717;
  }
  .pricing-controls.home button { color: #a3a3a3; }
  .pricing-controls.home button.active {
    background: white;
    color: #050505;
    box-shadow: none;
  }
  .pricing-controls.home span { color: #74aaff; }
  .home-pricing-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: clamp(24px, 7vw, 92px);
  }
  .home-pricing-grid article,
  .home-pricing-grid article.featured {
    min-height: 405px;
    padding: 0 30px;
    border: 0;
    border-left: 1px solid rgb(255 255 255 / .78);
    border-radius: 0;
    background: transparent;
    color: white;
    box-shadow: none;
  }
  .home-plan-header {
    min-height: 127px;
    padding-bottom: 30px;
    border-bottom: 1px solid rgb(255 255 255 / .42);
  }
  .home-plan-title {
    min-height: 30px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 14px;
  }
  .home-plan-title h3 {
    color: white;
    font-family: var(--m-font-display);
    font-size: 23px;
    font-weight: 600;
    letter-spacing: -.025em;
  }
  .home-plan-title .badge {
    flex: none;
    margin: 0;
    padding: 5px 9px;
    border-radius: 5px;
    background: #242424;
    color: #d8d8d8;
    font: 600 10px/1 var(--font-mono);
    letter-spacing: .08em;
    text-transform: uppercase;
  }
  .home-plan-header p {
    min-height: 48px;
    margin: 10px 0 0;
    color: #8b8b8b;
    font-size: 15px;
    line-height: 1.5;
  }
  .home-plan-cost {
    flex: 1;
    padding-top: 40px;
  }
  .home-plan-cost > div {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
  }
  .home-plan-cost strong {
    color: white;
    font-family: var(--m-font-editorial);
    font-size: clamp(45px, 4.5vw, 62px);
    font-weight: 400;
    letter-spacing: -.045em;
    line-height: .95;
  }
  .home-plan-cost > div > span {
    margin-left: 2px;
    color: white;
    font-family: var(--m-font-editorial);
    font-size: clamp(24px, 2.3vw, 34px);
    letter-spacing: -.035em;
  }
  .home-plan-cost small {
    min-height: 20px;
    display: block;
    margin-top: 10px;
    color: #777;
    font-size: 11px;
  }
  .home-plan-cost p {
    margin: 22px 0 0;
    color: #aaa;
    font-size: 12px;
  }
  .home-plan-cost p span { color: #8b8b8b; }
  .home-plan-cta {
    width: fit-content;
    min-height: 48px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    align-self: flex-start;
    padding: 0 23px;
    border-radius: 99px;
    background: white;
    color: #006aff;
    font-size: 14px;
    font-weight: 650;
    transition:
      transform 180ms ease,
      background-color 180ms ease;
  }
  .home-plan-cta.primary { background: #006aff; color: white; }
  .home-plan-cta:hover { transform: translateY(-1px); background: #e7efff; }
  .home-plan-cta.primary:hover { background: #1878ff; }
  @media (max-width: 1050px) {
    .pricing-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .home-pricing-grid {
      gap: 12px;
    }
    .home-pricing-grid article,
    .home-pricing-grid article.featured { padding-inline: 22px; }
  }
  @media (max-width: 720px) {
    .pricing-controls.home { margin: 48px 0 28px; }
    .home-pricing-grid { grid-template-columns: 1fr; gap: 0; }
    .home-pricing-grid article,
    .home-pricing-grid article.featured {
      min-height: 0;
      padding: 36px 0;
      border-top: 1px solid rgb(255 255 255 / .45);
      border-left: 0;
    }
    .home-plan-header { min-height: 0; }
    .home-plan-cost { padding-block: 34px; }
    .home-plan-cost strong { font-size: 56px; }
    .home-plan-cost > div > span { font-size: 30px; }
    .plan-table-wrap { display: none; }
    .mobile-plans { display: grid; grid-template-columns: 1fr; }
  }
  @media (max-width: 600px) {
    .pricing-grid { grid-template-columns: 1fr; }
  }
</style>
