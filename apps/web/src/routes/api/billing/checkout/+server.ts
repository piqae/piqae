import { authKit } from '@workos/authkit-sveltekit';
import { error, json } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode } from '$lib/server/auth-config';
import {
  canManageHostedBilling,
  checkoutAllowed,
  checkoutIdempotencyKey,
  findStripeCustomer,
  paidPlans,
  stripeClient,
  stripeOveragePriceLookupKey,
  stripeOveragePriceMatchesCatalog,
  stripeCustomerIdempotencyKey,
  stripePriceLookupKey,
  stripePriceMatchesCatalog,
  subscriptionBlocksCheckout
} from '$lib/server/billing';
import { dashboardConnection } from '$lib/server/dashboard-data';
import type { BillingInterval, PlanSlug } from '$lib/marketing/types';

const intervals: BillingInterval[] = ['monthly', 'annual'];

export const POST: RequestHandler = async (event) => {
  const origin = event.request.headers.get('origin');
  if (origin !== event.url.origin) error(403, 'Cross-origin checkout requests are denied');
  if (authMode !== 'workos') error(409, 'Managed Cloud billing is available only in hosted mode');

  const user = await authKit.getUser(event);
  if (!user) error(401, 'Authentication is required');
  if (!canManageHostedBilling(event.locals.auth?.role)) {
    error(403, 'Billing access denied');
  }
  const workosOrganizationId = event.locals.auth?.organizationId;
  if (!workosOrganizationId) error(409, 'Choose or create a workspace before starting checkout');

  let raw: unknown;
  try {
    raw = await event.request.json();
  } catch {
    error(400, 'Invalid checkout request');
  }
  if (!raw || typeof raw !== 'object') error(400, 'Invalid checkout request');
  const { plan, interval } = raw as { plan?: unknown; interval?: unknown };
  if (!paidPlans.includes(plan as PlanSlug) || !intervals.includes(interval as BillingInterval)) {
    error(400, 'Invalid plan or billing interval');
  }
  if (!checkoutAllowed(plan as PlanSlug, interval as BillingInterval)) {
    error(409, 'That plan and interval are not available for checkout');
  }
  const lookupKey = stripePriceLookupKey(plan as PlanSlug, interval as BillingInterval);
  const overageLookupKey = stripeOveragePriceLookupKey(
    plan as PlanSlug,
    interval as BillingInterval
  );
  if (!lookupKey || !overageLookupKey) {
    error(503, 'The selected Stripe prices are not configured');
  }

  const stripe = stripeClient();
  const [prices, overagePrices] = await Promise.all([
    stripe.prices.list({ active: true, lookup_keys: [lookupKey], limit: 2 }),
    stripe.prices.list({ active: true, lookup_keys: [overageLookupKey], limit: 2 })
  ]);
  if (prices.data.length !== 1 || overagePrices.data.length !== 1) {
    error(503, 'The selected Stripe prices are unavailable');
  }
  const price = prices.data[0];
  const overagePrice = overagePrices.data[0];
  if (!price || !overagePrice) error(503, 'The selected Stripe prices are unavailable');
  if (!stripePriceMatchesCatalog(price, plan as PlanSlug, interval as BillingInterval)) {
    error(409, 'The selected Stripe price does not match the Piqae pricing catalog');
  }
  if (
    !stripeOveragePriceMatchesCatalog(
      overagePrice,
      plan as PlanSlug,
      interval as BillingInterval
    )
  ) {
    error(409, 'The selected Stripe overage price does not match the Piqae pricing catalog');
  }

  const { baseUrl, bearerToken } = dashboardConnection(event);
  const workspaceResponse = await event.fetch(
    `${baseUrl.replace(/\/$/, '')}/v1/workspaces/current`,
    {
      headers: {
        accept: 'application/json',
        authorization: `Bearer ${bearerToken}`,
        'x-piqae-dashboard': '1'
      }
    }
  );
  if (!workspaceResponse.ok) {
    error(409, 'The current Piqae workspace could not be verified');
  }
  const workspace = (await workspaceResponse.json()) as { id?: unknown };
  if (typeof workspace.id !== 'string' || workspace.id === '') {
    error(409, 'The current Piqae workspace identity is invalid');
  }
  const workspaceId = workspace.id;
  const existingCustomer = await findStripeCustomer(stripe, workspaceId);
  if (existingCustomer) {
    const subscriptions = await stripe.subscriptions.list({
      customer: existingCustomer.id,
      status: 'all',
      limit: 100
    });
    if (
      subscriptions.has_more ||
      subscriptions.data.some((subscription) => subscriptionBlocksCheckout(subscription.status))
    ) {
      error(409, 'This workspace already has a managed subscription; use the billing portal');
    }
  }
  const customer =
    existingCustomer ??
    (await stripe.customers.create(
      {
        email: user.email,
        metadata: {
          workspace_id: workspaceId,
          workos_organization_id: workosOrganizationId
        }
      },
      { idempotencyKey: stripeCustomerIdempotencyKey(workspaceId) }
    ));

  const session = await stripe.checkout.sessions.create(
    {
      mode: 'subscription',
      line_items: [
        { price: price.id, quantity: 1 },
        { price: overagePrice.id }
      ],
      client_reference_id: workspaceId,
      customer: customer.id,
      allow_promotion_codes: false,
      success_url: `${event.url.origin}/dashboard/settings?checkout=success#billing`,
      cancel_url: `${event.url.origin}/pricing?checkout=cancelled`,
      metadata: {
        workspace_id: workspaceId,
        workos_organization_id: workosOrganizationId,
        plan: String(plan),
        interval: String(interval)
      },
      subscription_data: {
        metadata: {
          workspace_id: workspaceId,
          workos_organization_id: workosOrganizationId,
          plan: String(plan),
          interval: String(interval),
          overage_price_id: overagePrice.id
        }
      }
    },
    {
      idempotencyKey: checkoutIdempotencyKey(
        workspaceId,
        plan as PlanSlug,
        interval as BillingInterval,
        [price.id, overagePrice.id]
      )
    }
  );
  if (!session.url) error(503, 'Stripe did not return a checkout URL');
  return json({ url: session.url }, { headers: { 'cache-control': 'no-store, private' } });
};
