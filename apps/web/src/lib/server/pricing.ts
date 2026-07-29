import type { CloudPricingCatalog, PricingDisplay } from '$lib/marketing/types';

const plans: PricingDisplay[] = [
  {
    plan: 'free',
    name: 'Free',
    currency: 'USD',
    monthlyCents: 0,
    annualCents: 0,
    headline: 'Prove the complete print path before paying.',
    includedAcceptedJobs: 100,
    annualIncludedAcceptedJobs: null,
    includedNodes: 1,
    workspaceMembers: 'unlimited',
    customerAccounts: 'not_included',
    jobOverageUnit: null,
    jobOverageCents: null,
    metadataRetentionDays: 7,
    documentRetention: {
      defaultHours: 24,
      maximumHours: 24,
      configurable: false,
      enforcement: 'preview_policy'
    },
    quotaBehavior: 'blocked',
    virtualTestJobs: 'unlimited',
    features: [
      'Test and Live environments',
      'API, SDK and webhooks',
      'Community support'
    ],
    cta: 'Start free'
  },
  {
    plan: 'pro',
    name: 'Pro',
    currency: 'USD',
    monthlyCents: 900,
    annualCents: 9_000,
    headline: 'Run production printing without complicated tiers.',
    includedAcceptedJobs: 25_000,
    annualIncludedAcceptedJobs: 300_000,
    includedNodes: 25,
    workspaceMembers: 'unlimited',
    customerAccounts: 'included',
    jobOverageUnit: 1_000,
    jobOverageCents: 25,
    metadataRetentionDays: 90,
    documentRetention: {
      defaultHours: 24,
      maximumHours: 168,
      configurable: true,
      enforcement: 'preview_policy'
    },
    quotaBehavior: 'overage',
    virtualTestJobs: 'unlimited',
    features: [
      'Platform customer accounts',
      'Configurable document retention policy (preview)',
      'Private-beta email support'
    ],
    badge: 'Production',
    cta: 'Choose Pro'
  }
];

export const cloudPricingCatalog: CloudPricingCatalog = {
  version: '2026-07-29',
  currency: 'USD',
  billableEvent: 'accepted_by_spooler',
  plans
};

export const paidPlans = ['pro'] as const;

export type PricingProse = Partial<Record<'free' | 'pro', { headline: string }>>;

export function pricingCatalog(prose: PricingProse = {}): CloudPricingCatalog {
  const catalog = structuredClone(cloudPricingCatalog);
  for (const plan of catalog.plans) {
    const headline = prose[plan.plan]?.headline.trim();
    if (headline) plan.headline = headline;
  }
  return catalog;
}

export function pricingPlan(plan: string): PricingDisplay | null {
  return cloudPricingCatalog.plans.find((item) => item.plan === plan) ?? null;
}
