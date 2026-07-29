<script lang="ts">
  import { browser } from '$app/environment';
  import MarketingShell from '$lib/components/marketing/MarketingShell.svelte';
  import Seo from '$lib/components/marketing/Seo.svelte';
  import {
    estimatePrintNode,
    estimateSpool
  } from '$lib/marketing/calculator';
  import { formatUsd } from '$lib/marketing/plans';
  import { safeExternalHttpUrl } from '$lib/marketing/urls';
  import type { BillingInterval } from '$lib/marketing/types';
  import { captureMarketingEvent } from '$lib/marketing/analytics';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();

  const params = browser ? new URLSearchParams(window.location.search) : new URLSearchParams();
  const bounded = (value: string | null, fallback: number, max: number) => {
    if (value === null) return fallback;
    const parsed = Number(value);
    return Number.isFinite(parsed) ? Math.max(0, Math.min(max, Math.round(parsed))) : fallback;
  };

  let jobs = $state(bounded(params.get('jobs'), 25_000, 100_000_000));
  let agents = $state(bounded(params.get('agents'), 3, 100_000));
  let tenants = $state(bounded(params.get('tenants'), 0, 100_000));
  let growthPercent = $state(bounded(params.get('growth'), 0, 500));
  let interval = $state<BillingInterval>(params.get('interval') === 'annual' ? 'annual' : 'monthly');

  let input = $derived({ jobs, agents, tenants, growthPercent, interval });
  let spool = $derived(estimateSpool(input, data.spoolPricing));
  let printNode = $derived(estimatePrintNode(input, data.printNodeSnapshot));
  let difference = $derived(
    spool.available ? printNode.monthlyCents - spool.monthlyCents : 0
  );
  let percent = $derived(
    printNode.monthlyCents > 0 ? Math.round((difference / printNode.monthlyCents) * 100) : 0
  );
  let snapshotExpired = $derived(
    new Date() > new Date(`${data.printNodeSnapshot.reviewDueAt}T23:59:59Z`)
  );
  let officialSource = $derived(safeExternalHttpUrl(data.printNodeSnapshot.sourceUrl));

  function shareEstimate() {
    if (!browser) return;
    const next = new URLSearchParams({
      jobs: String(jobs),
      agents: String(agents),
      tenants: String(tenants),
      growth: String(growthPercent),
      interval
    });
    history.replaceState({}, '', `${window.location.pathname}?${next}`);
    navigator.clipboard?.writeText(window.location.href);
    const jobBucket =
      jobs < 10_000 ? '<10k' : jobs < 100_000 ? '10k-100k' : jobs < 1_000_000 ? '100k-1m' : '1m+';
    const agentBucket = agents < 5 ? '<5' : agents < 50 ? '5-50' : agents < 500 ? '50-500' : '500+';
    captureMarketingEvent('cost_calculator_completed', {
      job_bucket: jobBucket,
      agent_bucket: agentBucket,
      has_tenants: tenants > 0,
      interval
    });
  }
</script>

<Seo
  title="PrintNode cost calculator — Compare with Spool"
  description="Estimate monthly and annual Spool and PrintNode list-price costs using jobs, connected agents, customer tenants, and expected growth."
  path="/tools/printnode-cost-calculator"
  noindex={snapshotExpired}
/>

<MarketingShell>
  <section class="m-page-hero">
    <div class="m-container">
      <span class="m-eyebrow">PrintNode cost calculator</span>
      <h1 class="m-title">Model the workflow, not just the headline tier.</h1>
      <p class="m-lede">
        Estimate public list-price costs from job volume, connected computers, customer tenants,
        billing interval, and expected growth. No inputs leave this browser.
      </p>
    </div>
  </section>

  <section class="calculator-section m-section-compact">
    <div class="m-container calculator">
      <form class="inputs" onsubmit={(event) => event.preventDefault()}>
        <label>
          <span>Jobs per month</span>
          <input type="number" min="0" max="100000000" step="1000" bind:value={jobs} />
        </label>
        <label>
          <span>Connected computers / agents</span>
          <input type="number" min="0" max="100000" bind:value={agents} />
        </label>
        <label>
          <span>Customer subaccounts / tenants</span>
          <input type="number" min="0" max="100000" bind:value={tenants} />
        </label>
        <label>
          <span>Expected growth</span>
          <div class="suffix"><input type="number" min="0" max="500" bind:value={growthPercent} /><i>%</i></div>
        </label>
        <fieldset>
          <legend>Spool billing interval</legend>
          <div class="interval">
            <button type="button" class:active={interval === 'monthly'} onclick={() => (interval = 'monthly')}>Monthly</button>
            <button type="button" class:active={interval === 'annual'} onclick={() => (interval = 'annual')}>Annual</button>
          </div>
        </fieldset>
        <button class="m-button" type="button" onclick={shareEstimate}>Copy shareable estimate</button>
      </form>

      <div class="results">
        {#if snapshotExpired}
          <div class="expired">
            <strong>Numeric comparison needs review.</strong>
            <p>The PrintNode price snapshot passed its 90-day review date, so results are hidden until a reviewer verifies the source.</p>
          </div>
        {:else if !spool.available}
          <div class="expired">
            <strong>Your fleet is outside the public Pro allowance.</strong>
            <p>{spool.note}</p>
            <a class="m-button" href="/pricing">Review the public plans</a>
          </div>
        {:else}
          <div class="result-head">
            <span>Estimated list price</span>
            <strong class:negative={difference < 0}>
              {difference >= 0 ? `${formatUsd(difference)} less / month` : `${formatUsd(Math.abs(difference))} more / month`}
            </strong>
            <small>{difference >= 0 ? `${percent}% below the modelled PrintNode cost` : 'PrintNode is lower for these inputs'}</small>
          </div>
          <div class="result-cards">
            <article>
              <div><span>Spool</span><small>{spool.plan}</small></div>
              <strong>{formatUsd(spool.monthlyCents)}<i>/mo</i></strong>
              <p>{formatUsd(spool.annualCents)} estimated annually</p>
            </article>
            <article>
              <div><span>PrintNode</span><small>{printNode.plan}</small></div>
              <strong>{formatUsd(printNode.monthlyCents)}<i>/mo</i></strong>
              <p>{formatUsd(printNode.annualCents)} estimated annually</p>
            </article>
          </div>
          <div class="assumptions">
            <p>{spool.note}</p>
            <p>{printNode.note}</p>
            <p>Expected growth is applied to job volume before selecting a plan. Taxes, negotiated terms, migration work, and support costs are excluded.</p>
          </div>
          <div class="m-actions">
            <a class="m-button primary" href="/start?plan=free&source=calculator">Start with Spool</a>
            <a class="m-button" href="/migrate/printnode">Plan the migration</a>
          </div>
        {/if}
      </div>
    </div>
  </section>

  <section class="sources m-section">
    <div class="m-narrow">
      <span class="m-eyebrow">Sources and limits</span>
      <h2 class="m-heading">An estimate, with its assumptions attached.</h2>
      <p>
        PrintNode values come from its
        {#if officialSource}
          <a href={officialSource} rel="noopener noreferrer">official USD pricing page</a>
        {:else}
          <span>stored pricing evidence</span>
        {/if}, observed
        {data.printNodeSnapshot.observedAt} and due for review
        {data.printNodeSnapshot.reviewDueAt}. Spool values come from server catalog
        {data.spoolPricing.version}.
      </p>
      <p>
        This calculator does not promise savings. It does not model discounts, taxes, foreign
        exchange, vendor price changes, unusual API behaviours, or the engineering cost of a
        migration. Verify both checkout totals before purchasing.
      </p>
    </div>
  </section>
</MarketingShell>

<style>
  .calculator-section { background: #eeece6; }
  .calculator { display: grid; grid-template-columns: 360px 1fr; gap: 12px; }
  .inputs, .results { padding: 26px; border: 1px solid var(--m-border); border-radius: 17px; background: rgb(255 255 255 / .65); }
  .inputs { display: grid; align-content: start; gap: 18px; }
  label, fieldset { display: grid; gap: 7px; padding: 0; border: 0; }
  label > span, legend { color: var(--m-muted); font-size: 11px; font-weight: 650; }
  input { width: 100%; height: 46px; padding: 0 12px; border: 1px solid var(--m-border); border-radius: 9px; background: white; color: var(--m-ink); font: 14px var(--font-mono); }
  .suffix { position: relative; }
  .suffix i { position: absolute; right: 12px; top: 12px; color: var(--m-faint); font-style: normal; }
  .interval { display: grid; grid-template-columns: 1fr 1fr; gap: 3px; padding: 4px; border: 1px solid var(--m-border); border-radius: 9px; }
  .interval button { height: 34px; border: 0; border-radius: 6px; background: transparent; color: var(--m-muted); cursor: pointer; }
  .interval button.active { background: white; color: var(--m-ink); box-shadow: 0 1px 4px rgb(23 22 27 / .08); }
  .results { min-height: 560px; }
  .result-head { display: grid; padding-bottom: 30px; border-bottom: 1px solid var(--m-border); }
  .result-head > span { color: var(--m-faint); font-size: 11px; text-transform: uppercase; letter-spacing: .05em; }
  .result-head > strong { margin-top: 16px; color: var(--m-green); font-size: clamp(31px, 5vw, 52px); letter-spacing: -.055em; line-height: 1; }
  .result-head > strong.negative { color: #b86654; }
  .result-head small { margin-top: 8px; color: var(--m-muted); }
  .result-cards { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; margin-top: 22px; }
  .result-cards article { padding: 20px; border: 1px solid var(--m-border); border-radius: 12px; background: white; }
  .result-cards article div { display: flex; justify-content: space-between; }
  .result-cards small { color: var(--m-violet-dark); }
  .result-cards strong { display: block; margin-top: 35px; font-size: 31px; letter-spacing: -.04em; }
  .result-cards strong i { color: var(--m-faint); font-size: 11px; font-style: normal; font-weight: 500; letter-spacing: 0; }
  .result-cards p, .assumptions { color: var(--m-faint); font-size: 10px; }
  .assumptions { margin-top: 20px; }
  .assumptions p { margin: 6px 0; }
  .expired { padding: 26px; border: 1px solid #b8865c; border-radius: 12px; background: #fff5e8; }
  .expired p { color: var(--m-muted); }
  .sources p { color: var(--m-muted); font-size: 17px; }
  .sources a { color: var(--m-violet-dark); text-decoration: underline; text-underline-offset: 3px; }
  @media (max-width: 800px) { .calculator { grid-template-columns: 1fr; } .results { min-height: 0; } }
  @media (max-width: 560px) { .result-cards { grid-template-columns: 1fr; } }
</style>
