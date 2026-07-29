import type Stripe from 'stripe';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const { privateEnvironment } = vi.hoisted(() => ({
  privateEnvironment: {} as Record<string, string>
}));

vi.mock('$env/dynamic/private', () => ({ env: privateEnvironment }));

import {
  canManageHostedBilling,
  checkoutAllowed,
  checkoutIdempotencyKey,
  parseBillingSummary,
  stripeCustomerIdempotencyKey,
  stripeOveragePriceMatchesCatalog,
  stripePriceMatchesCatalog,
  subscriptionBlocksCheckout
} from './billing';

function price(overrides: Partial<Stripe.Price> = {}): Stripe.Price {
  return {
    id: 'price_base',
    object: 'price',
    active: true,
    billing_scheme: 'per_unit',
    currency: 'usd',
    metadata: {},
    product: 'prod_spool',
    recurring: {
      interval: 'month',
      interval_count: 1,
      meter: null,
      trial_period_days: null,
      usage_type: 'licensed'
    },
    tax_behavior: 'unspecified',
    tiers_mode: null,
    transform_quantity: null,
    type: 'recurring',
    unit_amount: 900,
    unit_amount_decimal: '900' as never,
    custom_unit_amount: null,
    livemode: false,
    lookup_key: null,
    nickname: null,
    created: 0,
    tiers: undefined,
    currency_options: undefined,
    ...overrides
  };
}

describe('hosted billing contract', () => {
  beforeEach(() => {
    for (const key of Object.keys(privateEnvironment)) delete privateEnvironment[key];
  });

  it('limits hosted billing mutations to the locked billing roles', () => {
    expect(canManageHostedBilling('owner')).toBe(true);
    expect(canManageHostedBilling('billing')).toBe(true);
    for (const role of ['admin', 'developer', 'operator', 'viewer', null]) {
      expect(canManageHostedBilling(role)).toBe(false);
    }
  });

  it('enables checkout only when base and metered overage prices are configured', () => {
    privateEnvironment.STRIPE_CHECKOUT_ENABLED = 'true';
    privateEnvironment.STRIPE_PRICE_PRO_MONTHLY = 'spool_pro_monthly';

    expect(checkoutAllowed('pro', 'monthly')).toBe(false);
    privateEnvironment.STRIPE_PRICE_PRO_OVERAGE_MONTHLY = 'spool_pro_overage_monthly';
    expect(checkoutAllowed('pro', 'monthly')).toBe(true);
  });

  it('requires the exact licensed base price', () => {
    expect(stripePriceMatchesCatalog(price(), 'pro', 'monthly')).toBe(true);
    expect(
      stripePriceMatchesCatalog(price({ unit_amount: 901 }), 'pro', 'monthly')
    ).toBe(false);
    expect(
      stripePriceMatchesCatalog(
        price({
          recurring: {
            interval: 'month',
            interval_count: 1,
            meter: 'mtr_wrong',
            trial_period_days: null,
            usage_type: 'metered'
          }
        }),
        'pro',
        'monthly'
      )
    ).toBe(false);
  });

  it('requires a metered overage price bound to the catalog usage contract', () => {
    const overage = price({
      id: 'price_overage',
      unit_amount: 25,
      metadata: {
        spool_plan: 'pro',
        spool_metric: 'accepted_live_jobs_overage',
        spool_included_jobs: '25000',
        spool_overage_unit: '1000'
      },
      recurring: {
        interval: 'month',
        interval_count: 1,
        meter: 'mtr_accepted_jobs',
        trial_period_days: null,
        usage_type: 'metered'
      }
    });

    expect(stripeOveragePriceMatchesCatalog(overage, 'pro', 'monthly')).toBe(true);
    expect(
      stripeOveragePriceMatchesCatalog(
        { ...overage, metadata: { ...overage.metadata, spool_overage_unit: '1' } },
        'pro',
        'monthly'
      )
    ).toBe(false);
  });

  it('requires the 300,000-job annual allowance on the annual meter', () => {
    const annualOverage = price({
      id: 'price_overage_annual',
      unit_amount: 25,
      metadata: {
        spool_plan: 'pro',
        spool_metric: 'accepted_live_jobs_overage',
        spool_included_jobs: '300000',
        spool_overage_unit: '1000'
      },
      recurring: {
        interval: 'year',
        interval_count: 1,
        meter: 'mtr_accepted_jobs_annual',
        trial_period_days: null,
        usage_type: 'metered'
      }
    });

    expect(stripeOveragePriceMatchesCatalog(annualOverage, 'pro', 'annual')).toBe(true);
    expect(
      stripeOveragePriceMatchesCatalog(
        {
          ...annualOverage,
          metadata: { ...annualOverage.metadata, spool_included_jobs: '25000' }
        },
        'pro',
        'annual'
      )
    ).toBe(false);
  });

  it('deduplicates repeat checkout requests without merging later attempts forever', () => {
    const prices = ['price_base', 'price_overage'];
    const first = checkoutIdempotencyKey(
      'workspace_123',
      'pro',
      'monthly',
      prices,
      1_800_000
    );
    expect(
      checkoutIdempotencyKey(
        'workspace_123',
        'pro',
        'monthly',
        [...prices].reverse(),
        1_800_100
      )
    ).toBe(first);
    expect(
      checkoutIdempotencyKey(
        'workspace_123',
        'pro',
        'monthly',
        prices,
        2_100_000
      )
    ).not.toBe(first);
  });

  it('uses one stable Stripe customer creation key per Spool workspace', () => {
    expect(stripeCustomerIdempotencyKey('workspace_123')).toBe(
      stripeCustomerIdempotencyKey('workspace_123')
    );
    expect(stripeCustomerIdempotencyKey('workspace_123')).not.toBe(
      stripeCustomerIdempotencyKey('workspace_456')
    );
    expect(stripeCustomerIdempotencyKey('workspace_123')).not.toContain('workspace_123');
  });

  it('prevents duplicate Checkout for every non-terminal Stripe subscription', () => {
    for (const status of ['incomplete', 'trialing', 'active', 'past_due', 'unpaid', 'paused']) {
      expect(subscriptionBlocksCheckout(status)).toBe(true);
    }
    expect(subscriptionBlocksCheckout('canceled')).toBe(false);
    expect(subscriptionBlocksCheckout('incomplete_expired')).toBe(false);
  });

  it('strictly projects billing and usage responses', () => {
    expect(
      parseBillingSummary({
        enabled: true,
        managed_by_platform: false,
        plan: 'pro',
        billing_interval: 'monthly',
        subscription_status: 'active',
        grace_ends_at: null,
        accept_new_cloud_jobs: true,
        entitlement: {
          included_live_jobs: 25_000,
          node_limit: 25,
          metadata_retention_days: 90,
          document_retention_hours: 168,
          overage_job_unit: 1_000,
          overage_price_cents: 25
        },
        usage: {
          period_start: '2026-07-01T00:00:00Z',
          period_end: '2026-08-01T00:00:00Z',
          accepted_live_jobs: 26_000,
          active_nodes: 2
        },
        overage_live_jobs: 1_000
      })
    ).toMatchObject({
      plan: 'pro',
      billingInterval: 'monthly',
      entitlement: { includedLiveJobs: 25_000, nodeLimit: 25 },
      usage: { acceptedLiveJobs: 26_000, activeNodes: 2 },
      overageLiveJobs: 1_000
    });

    expect(() =>
      parseBillingSummary({
        enabled: true,
        managed_by_platform: false,
        plan: 'free',
        billing_interval: null,
        subscription_status: null,
        grace_ends_at: null,
        accept_new_cloud_jobs: true,
        usage: {
          period_start: '2026-07-01T00:00:00Z',
          period_end: '2026-08-01T00:00:00Z',
          accepted_live_jobs: -1,
          active_nodes: 0
        },
        overage_live_jobs: 0
      })
    ).toThrow(/invalid/);
  });
});
