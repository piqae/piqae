import { describe, expect, it } from 'vitest';
import { estimatePrintNode, estimatePiqae } from './calculator';
import { cloudPricingCatalog } from '$lib/server/pricing';

describe('marketing cost calculator', () => {
  it('selects the free Piqae plan for a small workflow', () => {
    expect(
      estimatePiqae(
        { jobs: 100, agents: 1, tenants: 0, growthPercent: 0, interval: 'monthly' },
        cloudPricingCatalog
      )
    ).toMatchObject({ plan: 'Free', monthlyCents: 0 });
  });

  it('uses the locked annual Pro price', () => {
    expect(
      estimatePiqae(
        {
          jobs: 25_000,
          agents: 25,
          tenants: 0,
          growthPercent: 0,
          interval: 'annual'
        },
        cloudPricingCatalog
      )
    ).toMatchObject({ plan: 'Pro', annualCents: 9_000, monthlyCents: 750 });
  });

  it('adds paid job overages to the annual Piqae total', () => {
    expect(
      estimatePiqae(
        {
          jobs: 27_000,
          agents: 8,
          tenants: 0,
          growthPercent: 0,
          interval: 'annual'
        },
        cloudPricingCatalog
      )
    ).toMatchObject({ plan: 'Pro', annualCents: 9_600, monthlyCents: 800 });
  });

  it('does not invent a public price above the Pro node allowance', () => {
    expect(
      estimatePiqae(
        {
          jobs: 25_000,
          agents: 26,
          tenants: 0,
          growthPercent: 0,
          interval: 'monthly'
        },
        cloudPricingCatalog
      )
    ).toMatchObject({ plan: 'Contact us', available: false });
  });

  it('selects an integrator plan when tenants are present', () => {
    expect(
      estimatePrintNode({
        jobs: 80_000,
        agents: 12,
        tenants: 10,
        growthPercent: 0,
        interval: 'monthly'
      }).plan
    ).toBe('Standard Integrator');
  });

  it('uses PrintNode annual single-account pricing when selected', () => {
    expect(
      estimatePrintNode({
        jobs: 20_000,
        agents: 5,
        tenants: 0,
        growthPercent: 0,
        interval: 'annual'
      })
    ).toMatchObject({ plan: 'Standard', annualCents: 29_000 });
  });

  it('applies documented Large Integrator job and subaccount overages', () => {
    expect(
      estimatePrintNode({
        jobs: 600_000,
        agents: 300,
        tenants: 250,
        growthPercent: 0,
        interval: 'monthly'
      })
    ).toMatchObject({ plan: 'Large Integrator', monthlyCents: 67_500 });
  });

  it('returns an unavailable estimate when no PrintNode tier can serve the input', () => {
    expect(
      estimatePrintNode(
        {
          jobs: 1,
          agents: 1,
          tenants: 1,
          growthPercent: 0,
          interval: 'monthly'
        },
        {
          currency: 'USD',
          sourceUrl: 'https://example.com/pricing',
          observedAt: '2026-07-29',
          reviewDueAt: '2026-10-27',
          tiers: []
        }
      )
    ).toMatchObject({ plan: 'Contact us', available: false });
  });
});
