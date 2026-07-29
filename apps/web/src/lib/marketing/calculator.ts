import type { BillingInterval, CloudPricingCatalog, PricingDisplay } from './types';

export interface CalculatorInput {
  jobs: number;
  agents: number;
  tenants: number;
  growthPercent: number;
  interval: BillingInterval;
}

export interface CostEstimate {
  plan: string;
  monthlyCents: number;
  annualCents: number;
  note: string;
  available: boolean;
}

export interface PrintNodeTier {
  name: string;
  baseMonthlyCents: number;
  annualBaseCents?: number;
  includedJobs: number;
  annualIncludedJobs?: number;
  includedComputers: number | null;
  includedSubaccounts: number;
  extraJobUnit: number;
  extraJobUnitCents: number;
  extraSubaccountCents?: number;
}

export interface PrintNodePricingSnapshot {
  currency: 'USD';
  sourceUrl: string;
  observedAt: string;
  reviewDueAt: string;
  tiers: PrintNodeTier[];
}

// Verified against PrintNode's public USD pricing page on 2026-07-29.
export const printNodePricingObservedAt = '2026-07-29';
export const printNodePricingReviewDueAt = '2026-10-27';

export const printNodeFallbackSnapshot: PrintNodePricingSnapshot = {
  currency: 'USD',
  sourceUrl: 'https://www.printnode.com/en/pricing',
  observedAt: printNodePricingObservedAt,
  reviewDueAt: printNodePricingReviewDueAt,
  tiers: [
    {
      name: 'Lite',
      baseMonthlyCents: 0,
      includedJobs: 50,
      includedComputers: 1,
      includedSubaccounts: 0,
      extraJobUnit: 0,
      extraJobUnitCents: 0
    },
    {
      name: 'Essential',
      baseMonthlyCents: 900,
      annualBaseCents: 9_000,
      includedJobs: 5_000,
      annualIncludedJobs: 60_000,
      includedComputers: 3,
      includedSubaccounts: 0,
      extraJobUnit: 1_000,
      extraJobUnitCents: 180
    },
    {
      name: 'Standard',
      baseMonthlyCents: 2_900,
      annualBaseCents: 29_000,
      includedJobs: 25_000,
      annualIncludedJobs: 300_000,
      includedComputers: 5,
      includedSubaccounts: 0,
      extraJobUnit: 1_000,
      extraJobUnitCents: 116
    },
    {
      name: 'Premium',
      baseMonthlyCents: 9_900,
      annualBaseCents: 99_000,
      includedJobs: 200_000,
      annualIncludedJobs: 2_400_000,
      includedComputers: null,
      includedSubaccounts: 0,
      extraJobUnit: 1_000,
      extraJobUnitCents: 49
    },
    {
      name: 'Standard Integrator',
      baseMonthlyCents: 6_000,
      includedJobs: 100_000,
      includedComputers: null,
      includedSubaccounts: 20,
      extraJobUnit: 1_000,
      extraJobUnitCents: 60,
      extraSubaccountCents: 300
    },
    {
      name: 'Large Integrator',
      baseMonthlyCents: 50_000,
      includedJobs: 500_000,
      includedComputers: null,
      includedSubaccounts: 200,
      extraJobUnit: 1_000,
      extraJobUnitCents: 50,
      extraSubaccountCents: 250
    }
  ]
};

function unitsOver(value: number, included: number, unit: number): number {
  if (value <= included || unit <= 0) return 0;
  return Math.ceil((value - included) / unit);
}

function spoolCostFor(
  plan: PricingDisplay,
  jobs: number,
  nodes: number,
  customerAccounts: number,
  interval: BillingInterval
): { monthlyCents: number; annualCents: number } | null {
  if (nodes > plan.includedNodes) return null;
  if (customerAccounts > 0 && plan.customerAccounts === 'not_included') return null;
  if (plan.plan === 'free') {
    return jobs <= plan.includedAcceptedJobs &&
      nodes <= plan.includedNodes &&
      customerAccounts === 0
      ? { monthlyCents: 0, annualCents: 0 }
      : null;
  }

  const annual = interval === 'annual';
  const measuredJobs = annual ? jobs * 12 : jobs;
  const includedJobs = annual
    ? (plan.annualIncludedAcceptedJobs ?? plan.includedAcceptedJobs * 12)
    : plan.includedAcceptedJobs;
  let cents = annual ? plan.annualCents : plan.monthlyCents;
  if (plan.jobOverageUnit && plan.jobOverageCents) {
    cents +=
      unitsOver(measuredJobs, includedJobs, plan.jobOverageUnit) *
      plan.jobOverageCents;
  } else if (measuredJobs > includedJobs) {
    return null;
  }
  return annual
    ? { monthlyCents: Math.round(cents / 12), annualCents: cents }
    : { monthlyCents: cents, annualCents: cents * 12 };
}

export function estimateSpool(
  input: CalculatorInput,
  catalog: CloudPricingCatalog
): CostEstimate {
  const jobs = Math.ceil(input.jobs * (1 + input.growthPercent / 100));
  const priced = catalog.plans
    .map((plan) => ({
      plan,
      cost: spoolCostFor(plan, jobs, input.agents, input.tenants, input.interval)
    }))
    .filter(
      (
        entry
      ): entry is {
        plan: PricingDisplay;
        cost: { monthlyCents: number; annualCents: number };
      } => entry.cost !== null
    );
  if (priced.length === 0) {
    return {
      plan: 'Contact us',
      monthlyCents: 0,
      annualCents: 0,
      available: false,
      note: 'The public Pro plan includes up to 25 nodes. Contact us for a larger managed fleet.'
    };
  }
  const selected = priced.reduce((best, entry) =>
    entry.cost.annualCents < best.cost.annualCents ? entry : best
  );
  const display = selected.plan;
  return {
    plan: display.name,
    monthlyCents: selected.cost.monthlyCents,
    annualCents: selected.cost.annualCents,
    available: true,
    note: `Based on Spool pricing catalog ${catalog.version}; Stripe confirms the final charge at checkout.`
  };
}

function printNodeTierCost(
  tier: PrintNodeTier,
  jobs: number,
  agents: number,
  tenants: number,
  interval: BillingInterval
): { monthlyCents: number; annualCents: number } | null {
  if (tier.baseMonthlyCents === 0) {
    return jobs <= tier.includedJobs &&
      agents <= (tier.includedComputers ?? agents) &&
      tenants === 0
      ? { monthlyCents: 0, annualCents: 0 }
      : null;
  }
  if (tier.includedComputers !== null && agents > tier.includedComputers) return null;
  if (tenants > 0 && tier.includedSubaccounts === 0) return null;
  const monthlyOverages =
    unitsOver(jobs, tier.includedJobs, tier.extraJobUnit) * tier.extraJobUnitCents +
    Math.max(0, tenants - tier.includedSubaccounts) * (tier.extraSubaccountCents ?? 0);
  if (interval === 'annual' && tier.annualBaseCents !== undefined) {
    const annualCents =
      tier.annualBaseCents +
      unitsOver(jobs * 12, tier.annualIncludedJobs ?? tier.includedJobs * 12, tier.extraJobUnit) *
        tier.extraJobUnitCents;
    return { monthlyCents: Math.round(annualCents / 12), annualCents };
  }
  const monthlyCents = tier.baseMonthlyCents + monthlyOverages;
  return { monthlyCents, annualCents: monthlyCents * 12 };
}

export function estimatePrintNode(
  input: CalculatorInput,
  snapshot: PrintNodePricingSnapshot = printNodeFallbackSnapshot
): CostEstimate {
  const jobs = Math.ceil(input.jobs * (1 + input.growthPercent / 100));
  const candidates = snapshot.tiers
    .map((tier) => ({
      tier,
      cost: printNodeTierCost(tier, jobs, input.agents, input.tenants, input.interval)
    }))
    .filter(
      (
        entry
      ): entry is {
        tier: PrintNodeTier;
        cost: { monthlyCents: number; annualCents: number };
      } => entry.cost !== null
    );
  const selected = candidates.reduce((best, entry) =>
    entry.cost.annualCents < best.cost.annualCents ? entry : best
  );
  return {
    plan: selected.tier.name,
    monthlyCents: selected.cost.monthlyCents,
    annualCents: selected.cost.annualCents,
    available: true,
    note: `Estimate from public pricing verified ${snapshot.observedAt}; negotiated terms and taxes are excluded.`
  };
}
