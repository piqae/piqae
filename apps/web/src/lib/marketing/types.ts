export type PlanSlug = 'free' | 'pro';
export type BillingInterval = 'monthly' | 'annual';

export interface PricingDisplay {
  plan: PlanSlug;
  name: string;
  currency: 'USD';
  monthlyCents: number;
  annualCents: number;
  headline: string;
  includedReportedCompleteJobs: number;
  annualIncludedReportedCompleteJobs: number | null;
  includedNodes: number;
  workspaceMembers: 'unlimited';
  customerAccounts: 'not_included' | 'included';
  jobOverageUnit: number | null;
  jobOverageCents: number | null;
  metadataRetentionDays: number;
  documentRetention: {
    defaultHours: number;
    maximumHours: number;
    configurable: boolean;
    enforcement: 'preview_policy';
  };
  quotaBehavior: 'blocked' | 'overage';
  virtualTestJobs: 'unlimited';
  features: string[];
  badge?: string;
  cta: string;
}

export interface CloudPricingCatalog {
  version: string;
  currency: 'USD';
  billableEvent: 'completed_reported';
  plans: PricingDisplay[];
}

export interface ComparisonClaim {
  competitor: string;
  claim: string;
  sourceUrl: string;
  observedAt: string;
  reviewDueAt: string;
  status: 'draft' | 'verified' | 'expired';
}

export interface ReleaseArtifact {
  platform: 'macos' | 'windows' | 'linux';
  architecture: string;
  version: string;
  channel: 'stable' | 'preview';
  downloadUrl: string | null;
  sha256: string | null;
  signed: boolean;
  publishedAt: string | null;
  minimumOs: string;
  releaseNotesUrl: string | null;
  supportTier: 'stable' | 'preview' | 'disabled';
  reason?: string;
}
