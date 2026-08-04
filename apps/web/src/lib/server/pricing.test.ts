import { describe, expect, it } from 'vitest';
import { cloudPricingCatalog, pricingCatalog } from './pricing';

describe('server-owned Cloud pricing catalog', () => {
  it('locks the paid private-beta commercial contract', () => {
    expect(cloudPricingCatalog).toMatchObject({
      billableEvent: 'completed_reported',
      currency: 'USD',
      plans: [
        {
          plan: 'free',
          monthlyCents: 0,
          annualCents: 0,
          includedReportedCompleteJobs: 100,
          annualIncludedReportedCompleteJobs: null,
          includedNodes: 1,
          metadataRetentionDays: 7,
          documentRetention: { maximumHours: 24, enforcement: 'preview_policy' },
          quotaBehavior: 'blocked'
        },
        {
          plan: 'pro',
          monthlyCents: 900,
          annualCents: 9_000,
          includedReportedCompleteJobs: 25_000,
          annualIncludedReportedCompleteJobs: 300_000,
          includedNodes: 25,
          jobOverageUnit: 1_000,
          jobOverageCents: 25,
          metadataRetentionDays: 90,
          documentRetention: { maximumHours: 168, enforcement: 'preview_policy' },
          quotaBehavior: 'overage'
        }
      ]
    });
  });

  it('allows CMS prose to change a headline but not transactional values', () => {
    const rendered = pricingCatalog({
      pro: { headline: 'A concise editorial headline.' }
    });

    expect(rendered.plans[1]?.headline).toBe('A concise editorial headline.');
    expect(rendered.plans[1]).toMatchObject({
      monthlyCents: 900,
      annualCents: 9_000,
      includedReportedCompleteJobs: 25_000,
      annualIncludedReportedCompleteJobs: 300_000,
      includedNodes: 25,
      jobOverageCents: 25
    });
    expect(cloudPricingCatalog.plans[1]?.headline).not.toBe(
      'A concise editorial headline.'
    );
  });
});
