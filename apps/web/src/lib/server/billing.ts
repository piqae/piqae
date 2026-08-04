import { env } from '$env/dynamic/private';
import { createHash } from 'node:crypto';
import Stripe from 'stripe';
import type { BillingInterval, PlanSlug } from '$lib/marketing/types';
import { cloudPricingCatalog, paidPlans as catalogPaidPlans } from '$lib/server/pricing';

export const paidPlans: PlanSlug[] = [...catalogPaidPlans];

export interface BillingSummary {
  enabled: boolean;
  managedByPlatform: boolean;
  plan: PlanSlug | null;
  billingInterval: BillingInterval | null;
  subscriptionStatus:
    | 'active'
    | 'trialing'
    | 'past_due'
    | 'unpaid'
    | 'paused'
    | 'cancelled'
    | null;
  graceEndsAt: string | null;
  acceptNewCloudJobs: boolean;
  entitlement: {
    includedLiveJobs: number;
    nodeLimit: number;
    metadataRetentionDays: number;
    documentRetentionHours: number;
    overageJobUnit: number | null;
    overagePriceCents: number | null;
  } | null;
  usage: {
    periodStart: string;
    periodEnd: string;
    reportedCompleteLiveJobs: number;
    activeNodes: number;
  };
  overageLiveJobs: number;
}

export type UsageSummary = BillingSummary['usage'];

export function canManageHostedBilling(role: string | null | undefined): boolean {
  return role === 'owner' || role === 'billing';
}

export function stripePriceLookupKey(plan: PlanSlug, interval: BillingInterval): string | null {
  const suffix = `${plan.toUpperCase()}_${interval.toUpperCase()}`;
  return env[`STRIPE_PRICE_${suffix}`] ?? null;
}

export function stripeOveragePriceLookupKey(
  plan: PlanSlug,
  interval: BillingInterval
): string | null {
  const suffix = `${plan.toUpperCase()}_OVERAGE_${interval.toUpperCase()}`;
  return env[`STRIPE_PRICE_${suffix}`] ?? null;
}

export function stripeClient(): Stripe {
  if (!env.STRIPE_SECRET_KEY) throw new Error('Stripe billing is not configured');
  return new Stripe(env.STRIPE_SECRET_KEY, { maxNetworkRetries: 2, timeout: 8_000 });
}

export async function findStripeCustomer(
  stripe: Stripe,
  workspaceId: string
): Promise<Stripe.Customer | null> {
  if (!/^[A-Za-z0-9_.:-]{1,160}$/.test(workspaceId)) {
    throw new Error('The Piqae workspace ID cannot be used for billing lookup.');
  }
  const result = await stripe.customers.search({
    query: `metadata['workspace_id']:'${workspaceId}'`,
    limit: 2
  });
  if (result.data.length > 1) {
    throw new Error('Multiple Stripe customers map to the same Piqae workspace.');
  }
  return result.data[0] ?? null;
}

export function checkoutAllowed(plan: PlanSlug, interval: BillingInterval): boolean {
  if (env.STRIPE_CHECKOUT_ENABLED !== 'true') return false;
  if (!paidPlans.includes(plan)) return false;
  return Boolean(
    stripePriceLookupKey(plan, interval) && stripeOveragePriceLookupKey(plan, interval)
  );
}

export function stripePortalAvailable(): boolean {
  return env.STRIPE_CHECKOUT_ENABLED === 'true' && Boolean(env.STRIPE_SECRET_KEY);
}

export function subscriptionBlocksCheckout(status: string): boolean {
  return !['canceled', 'incomplete_expired'].includes(status);
}

export function stripePriceMatchesCatalog(
  price: Stripe.Price,
  plan: PlanSlug,
  interval: BillingInterval
): boolean {
  const catalogPlan = cloudPricingCatalog.plans.find((item) => item.plan === plan);
  if (!catalogPlan) return false;
  const expectedAmount = interval === 'monthly' ? catalogPlan.monthlyCents : catalogPlan.annualCents;
  const expectedInterval = interval === 'monthly' ? 'month' : 'year';
  return (
    price.active &&
    price.type === 'recurring' &&
    price.currency === 'usd' &&
    price.billing_scheme === 'per_unit' &&
    price.unit_amount === expectedAmount &&
    price.recurring?.interval === expectedInterval &&
    price.recurring.interval_count === 1 &&
    price.recurring.usage_type === 'licensed'
  );
}

export function stripeOveragePriceMatchesCatalog(
  price: Stripe.Price,
  plan: PlanSlug,
  interval: BillingInterval
): boolean {
  const catalogPlan = cloudPricingCatalog.plans.find((item) => item.plan === plan);
  if (
    !catalogPlan ||
    catalogPlan.jobOverageCents === null ||
    catalogPlan.jobOverageUnit === null
  ) {
    return false;
  }
  const expectedInterval = interval === 'monthly' ? 'month' : 'year';
  const expectedIncludedJobs =
    interval === 'annual'
      ? catalogPlan.annualIncludedReportedCompleteJobs
      : catalogPlan.includedReportedCompleteJobs;
  if (expectedIncludedJobs === null) return false;
  const metadata = (key: string) => price.metadata[`piqae_${key}`];
  return (
    price.active &&
    price.type === 'recurring' &&
    price.currency === 'usd' &&
    price.billing_scheme === 'per_unit' &&
    price.unit_amount === catalogPlan.jobOverageCents &&
    price.recurring?.interval === expectedInterval &&
    price.recurring.interval_count === 1 &&
    price.recurring.usage_type === 'metered' &&
    typeof price.recurring.meter === 'string' &&
    price.recurring.meter.length > 0 &&
    metadata('plan') === plan &&
    metadata('metric') === 'reported_complete_live_jobs_overage' &&
    metadata('included_jobs') === String(expectedIncludedJobs) &&
    metadata('overage_unit') === String(catalogPlan.jobOverageUnit)
  );
}

export function checkoutIdempotencyKey(
  workspaceId: string,
  plan: PlanSlug,
  interval: BillingInterval,
  priceIds: readonly string[],
  now = Date.now()
): string {
  const fiveMinuteWindow = Math.floor(now / 300_000);
  const digest = createHash('sha256')
    .update(
      JSON.stringify({
        workspaceId,
        plan,
        interval,
        priceIds: [...priceIds].sort(),
        fiveMinuteWindow
      })
    )
    .digest('hex');
  return `piqae-checkout-${digest}`;
}

export function stripeCustomerIdempotencyKey(workspaceId: string): string {
  return `piqae-customer-${createHash('sha256').update(workspaceId).digest('hex')}`;
}

export function parseBillingSummary(value: unknown): BillingSummary {
  if (!isRecord(value) || !isRecord(value.usage)) {
    throw new Error('Piqae billing summary response was invalid.');
  }
  const entitlement = isRecord(value.entitlement) ? value.entitlement : null;
  const plan = value.plan === 'free' || value.plan === 'pro' ? value.plan : null;
  const billingInterval =
    value.billing_interval === 'monthly' || value.billing_interval === 'annual'
      ? value.billing_interval
      : null;
  if (
    value.billing_interval !== null &&
    value.billing_interval !== 'monthly' &&
    value.billing_interval !== 'annual'
  ) {
    throw new Error('Piqae billing summary response was invalid.');
  }
  const statuses = ['active', 'trialing', 'past_due', 'unpaid', 'paused', 'cancelled'];
  const subscriptionStatus = statuses.includes(String(value.subscription_status))
    ? (value.subscription_status as BillingSummary['subscriptionStatus'])
    : null;
  const reportedCompleteLiveJobs = value.usage.reported_complete_live_jobs;
  const requiredUsage = [
    value.usage.period_start,
    value.usage.period_end,
    reportedCompleteLiveJobs,
    value.usage.active_nodes
  ];
  if (
    typeof value.enabled !== 'boolean' ||
    typeof value.managed_by_platform !== 'boolean' ||
    typeof value.accept_new_cloud_jobs !== 'boolean' ||
    !Number.isInteger(value.overage_live_jobs) ||
    requiredUsage.some((item) => item === undefined)
  ) {
    throw new Error('Piqae billing summary response was invalid.');
  }
  const mappedEntitlement = entitlement
    ? {
        includedLiveJobs: integer(entitlement.included_live_jobs),
        nodeLimit: integer(entitlement.node_limit),
        metadataRetentionDays: integer(entitlement.metadata_retention_days),
        documentRetentionHours: integer(entitlement.document_retention_hours),
        overageJobUnit: nullableInteger(entitlement.overage_job_unit),
        overagePriceCents: nullableInteger(entitlement.overage_price_cents)
      }
    : null;
  return {
    enabled: value.enabled,
    managedByPlatform: value.managed_by_platform,
    plan,
    billingInterval,
    subscriptionStatus,
    graceEndsAt: typeof value.grace_ends_at === 'string' ? value.grace_ends_at : null,
    acceptNewCloudJobs: value.accept_new_cloud_jobs,
    entitlement: mappedEntitlement,
    usage: parseUsageSummary(value.usage),
    overageLiveJobs: integer(value.overage_live_jobs)
  };
}

export function parseUsageSummary(value: unknown): UsageSummary {
  if (!isRecord(value)) throw new Error('Piqae usage response was invalid.');
  const reportedCompleteLiveJobs = value.reported_complete_live_jobs;
  return {
    periodStart: stringValue(value.period_start),
    periodEnd: stringValue(value.period_end),
    reportedCompleteLiveJobs: integer(reportedCompleteLiveJobs),
    activeNodes: integer(value.active_nodes)
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function integer(value: unknown): number {
  if (!Number.isInteger(value) || (value as number) < 0) {
    throw new Error('Piqae billing summary response was invalid.');
  }
  return value as number;
}

function nullableInteger(value: unknown): number | null {
  return value === null ? null : integer(value);
}

function stringValue(value: unknown): string {
  if (typeof value !== 'string' || value === '') {
    throw new Error('Piqae billing summary response was invalid.');
  }
  return value;
}
