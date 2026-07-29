import { env } from '$env/dynamic/private';
import { error, json } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import {
  paidPlans,
  stripeClient,
  stripeOveragePriceLookupKey,
  stripeOveragePriceMatchesCatalog,
  stripePriceLookupKey,
  stripePriceMatchesCatalog
} from '$lib/server/billing';
import type { BillingInterval } from '$lib/marketing/types';

export const GET: RequestHandler = async ({ request }) => {
  const expected = env.PRICING_DRIFT_SHARED_SECRET;
  if (!expected || request.headers.get('authorization') !== `Bearer ${expected}`) {
    error(401, 'Unauthorized');
  }
  const stripe = stripeClient();
  const details: Array<{
    plan: string;
    interval: BillingInterval;
    component: 'base' | 'overage';
    status: string;
  }> = [];

  for (const plan of paidPlans) {
    const intervals: BillingInterval[] = ['monthly', 'annual'];
    for (const interval of intervals) {
      const lookupKey = stripePriceLookupKey(plan, interval);
      if (!lookupKey) {
        details.push({ plan, interval, component: 'base', status: 'missing_configuration' });
      } else {
        const prices = await stripe.prices.list({
          active: true,
          lookup_keys: [lookupKey],
          limit: 2
        });
        const price = prices.data.length === 1 ? prices.data[0] : null;
        details.push({
          plan,
          interval,
          component: 'base',
          status:
            price && stripePriceMatchesCatalog(price, plan, interval) ? 'match' : 'mismatch'
        });
      }
      const overageLookupKey = stripeOveragePriceLookupKey(plan, interval);
      if (!overageLookupKey) {
        details.push({ plan, interval, component: 'overage', status: 'missing_configuration' });
      } else {
        const prices = await stripe.prices.list({
          active: true,
          lookup_keys: [overageLookupKey],
          limit: 2
        });
        const price = prices.data.length === 1 ? prices.data[0] : null;
        details.push({
          plan,
          interval,
          component: 'overage',
          status:
            price && stripeOveragePriceMatchesCatalog(price, plan, interval)
              ? 'match'
              : 'mismatch'
        });
      }
    }
  }
  const drift = details.some((item) => item.status !== 'match');
  return json({ drift, details }, { status: drift ? 409 : 200 });
};
