<script lang="ts">
  import DataError from '$lib/components/DataError.svelte';
  import PageHeader from '$lib/components/PageHeader.svelte';
  import { formatUsd } from '$lib/marketing/plans';
  import type { BillingInterval } from '$lib/marketing/types';
  import type { PageData } from './$types';
  import { untrack } from 'svelte';

  let { data }: { data: PageData } = $props();
  const initialInterval = untrack(() => data.selectedInterval);
  const initialCheckoutState = untrack(() => data.checkoutState);
  let interval = $state<BillingInterval>(initialInterval);
  let busy = $state(false);
  let message = $state(
    initialCheckoutState === 'success'
      ? 'Checkout returned successfully. Access changes after the billing webhook confirms the subscription.'
      : ''
  );

  const pro = $derived(data.pricing.plans.find((plan) => plan.plan === 'pro')!);
  const usagePercent = $derived(
    data.summary?.entitlement
      ? Math.min(
          100,
          Math.round(
            (data.summary.usage.acceptedLiveJobs /
              Math.max(1, data.summary.entitlement.includedLiveJobs)) *
              100
          )
        )
      : 0
  );

  async function openBilling(path: 'checkout' | 'portal') {
    busy = true;
    message = '';
    try {
      const response = await fetch(`/api/billing/${path}`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: path === 'checkout' ? JSON.stringify({ plan: 'pro', interval }) : '{}'
      });
      const body = (await response.json()) as { url?: string; message?: string };
      if (!response.ok || !body.url) {
        message = body.message ?? 'Managed billing is not available for this workspace.';
        return;
      }
      window.location.assign(body.url);
    } catch {
      message = 'Managed billing could not be opened. Try again or contact support.';
    } finally {
      busy = false;
    }
  }
</script>

<svelte:head>
  <title>Billing · Spool</title>
  <meta name="robots" content="noindex,nofollow" />
</svelte:head>

<PageHeader
  eyebrow="Workspace"
  title="Billing"
  description="Choose a managed Cloud plan. Stripe confirms the final amount before purchase."
/>

{#if message}<div class="notice" role="status">{message}</div>{/if}

{#if !data.available}
  <section class="panel unavailable">
    <strong>Managed billing is not enabled</strong>
    <p>Self-hosted and local-only deployments do not call Stripe or enforce Cloud plan limits.</p>
  </section>
{:else}
  {#if data.dataError}<DataError error={data.dataError} />{/if}

  <div class="billing-grid">
    <section class="panel">
      <header>
        <h2>Current plan</h2>
        <p>Server-projected subscription and immutable accepted-job usage.</p>
      </header>
      {#if data.summary}
        <div class="summary">
          <div class="plan-name">
            <span>{data.summary.plan === 'pro' ? 'Pro' : 'Free'}</span>
            <strong>
              {data.summary.subscriptionStatus ?? 'No paid subscription'}
              {#if data.summary.billingInterval} · {data.summary.billingInterval}{/if}
            </strong>
          </div>
          <div class="usage-row">
            <span>Accepted Live jobs</span>
            <strong>
              {data.summary.usage.acceptedLiveJobs.toLocaleString('en-US')}
              {#if data.summary.entitlement}
                / {data.summary.entitlement.includedLiveJobs.toLocaleString('en-US')}
              {/if}
            </strong>
          </div>
          <div class="meter" aria-label={`${usagePercent}% of included jobs used`}>
            <span style={`width: ${usagePercent}%`}></span>
          </div>
          <div class="usage-row">
            <span>Active nodes</span>
            <strong>
              {data.summary.usage.activeNodes}
              {#if data.summary.entitlement} / {data.summary.entitlement.nodeLimit}{/if}
            </strong>
          </div>
          {#if data.workspaceUsage && data.summary.managedByPlatform}
            <div class="usage-row">
              <span>This workspace’s accepted jobs</span>
              <strong>{data.workspaceUsage.acceptedLiveJobs.toLocaleString('en-US')}</strong>
            </div>
          {/if}
          <div class="usage-row">
            <span>New Cloud jobs</span>
            <strong>{data.summary.acceptNewCloudJobs ? 'Accepted' : 'Blocked'}</strong>
          </div>
          {#if data.summary.overageLiveJobs > 0}
            <div class="usage-row">
              <span>Overage jobs</span>
              <strong>{data.summary.overageLiveJobs.toLocaleString('en-US')}</strong>
            </div>
          {/if}
        </div>
      {:else}
        <div class="summary muted">Billing projection is temporarily unavailable.</div>
      {/if}
      <footer>
        <span>
          {data.summary?.managedByPlatform
            ? 'This workspace is billed through its owning platform.'
            : 'Taxes and the final charge are shown by Stripe.'}
        </span>
        {#if data.canManageBilling && data.portalAvailable && data.summary && !data.summary.managedByPlatform && data.summary.plan === 'pro'}
          <button class="button" disabled={busy} onclick={() => openBilling('portal')}>
            {busy ? 'Opening…' : 'Manage billing'}
          </button>
        {:else if data.canManageBilling && data.summary && !data.summary.managedByPlatform && data.summary.plan === 'free'}
          {#if data.checkoutAvailable[interval]}
            <button class="button primary" disabled={busy} onclick={() => openBilling('checkout')}>
              {busy ? 'Opening…' : 'Upgrade to Pro'}
            </button>
          {:else}
            <span>Checkout for this billing interval is not configured.</span>
          {/if}
        {:else if data.summary && !data.summary.managedByPlatform && !data.canManageBilling}
          <span>Only workspace owners and billing members can change the plan.</span>
        {:else if data.canManageBilling && data.summary && !data.summary.managedByPlatform}
          <span>Managed billing is not configured for this deployment. Contact support.</span>
        {:else if !data.summary}
          <span>Refresh after the billing projection becomes available. No billing action is safe while status is unknown.</span>
        {/if}
      </footer>
    </section>

    <aside class="panel">
      <header><h2>Pro</h2><p>Catalog {data.pricing.version}</p></header>
      <div class="explanation">
        <strong>{formatUsd(pro.monthlyCents)} monthly · {formatUsd(pro.annualCents)} annually</strong>
        <p>
          {(interval === 'annual'
            ? pro.annualIncludedAcceptedJobs
            : pro.includedAcceptedJobs)?.toLocaleString('en-US')}
          accepted jobs per {interval === 'annual' ? 'annual billing period' : 'month'} and
          {pro.includedNodes} nodes.
        </p>
        <strong>{formatUsd(pro.jobOverageCents ?? 0)} per 1,000 extra jobs</strong>
        <p>Overage keeps Pro printing; Free rejects new jobs at its limit.</p>
        <strong>Usage boundary</strong>
        <p>Spooler acceptance is billable once, but does not prove physical delivery.</p>
      </div>
      <div class="interval-picker" role="group" aria-label="Billing interval">
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
        >Annual</button>
      </div>
    </aside>
  </div>
{/if}

<style>
  .notice {
    margin-top: 14px;
    padding: 10px 12px;
    color: var(--info);
    background: var(--info-soft);
    border: 1px solid color-mix(in oklch, var(--info), transparent 70%);
    border-radius: var(--radius-md);
    font-size: 10px;
  }
  .billing-grid { display: grid; grid-template-columns: minmax(0, 620px) 260px; gap: 12px; padding-top: 18px; }
  .unavailable { display: grid; gap: 4px; margin-top: 18px; padding: 18px; }
  .unavailable strong { font-size: 11px; }
  .unavailable p { margin: 0; color: var(--text-tertiary); font-size: 10px; }
  section > header, aside > header { padding: 13px 14px; border-bottom: 1px solid var(--border-subtle); }
  h2 { margin: 0; font-size: 11px; font-weight: 550; }
  header p { margin: 3px 0 0; color: var(--text-tertiary); font-size: 9px; }
  .summary { display: grid; gap: 10px; padding: 14px; }
  .plan-name, .usage-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
  .plan-name span { font-size: 18px; font-weight: 600; }
  .plan-name strong, .usage-row strong { font-size: 10px; font-weight: 550; }
  .usage-row span { color: var(--text-tertiary); font-size: 10px; }
  .meter { height: 5px; overflow: hidden; background: var(--surface-raised); border-radius: 99px; }
  .meter span { height: 100%; display: block; background: var(--accent); border-radius: inherit; }
  footer {
    display: flex; align-items: center; justify-content: space-between; gap: 18px;
    padding: 10px 14px; border-top: 1px solid var(--border-subtle);
  }
  footer span { color: var(--text-tertiary); font-size: 9px; }
  .explanation { display: grid; gap: 3px; padding: 14px; }
  .explanation strong { margin-top: 10px; font-size: 10px; }
  .explanation strong:first-child { margin-top: 0; }
  .explanation p { margin: 0; color: var(--text-tertiary); font-size: 9px; line-height: 14px; }
  .interval-picker { display: grid; grid-template-columns: 1fr 1fr; gap: 3px; margin: 0 14px 14px; padding: 3px; background: var(--surface-raised); border-radius: 7px; }
  .interval-picker button { height: 27px; color: var(--text-tertiary); background: transparent; border: 0; border-radius: 5px; font-size: 9px; }
  .interval-picker button.active { color: var(--text-primary); background: var(--surface); }
  @media (max-width: 800px) {
    .billing-grid { grid-template-columns: 1fr; }
  }
  @media (max-width: 560px) {
    footer { align-items: stretch; flex-direction: column; }
  }
</style>
