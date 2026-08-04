import { authKit } from '@workos/authkit-sveltekit';
import { error, json } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode } from '$lib/server/auth-config';
import {
  canManageHostedBilling,
  findStripeCustomer,
  stripeClient
} from '$lib/server/billing';
import { dashboardConnection } from '$lib/server/dashboard-data';

export const POST: RequestHandler = async (event) => {
  if (event.request.headers.get('origin') !== event.url.origin) {
    error(403, 'Cross-origin billing portal requests are denied');
  }
  if (authMode !== 'workos') {
    error(409, 'Managed Cloud billing is available only in hosted mode');
  }
  const user = await authKit.getUser(event);
  if (!user) error(401, 'Authentication is required');
  if (!canManageHostedBilling(event.locals.auth?.role)) {
    error(403, 'Billing access denied');
  }
  if (!event.locals.auth?.organizationId) {
    error(409, 'Choose a workspace before opening the billing portal');
  }

  const { baseUrl, bearerToken } = dashboardConnection(event);
  const response = await event.fetch(`${baseUrl.replace(/\/$/, '')}/v1/workspaces/current`, {
    headers: {
      accept: 'application/json',
      authorization: `Bearer ${bearerToken}`,
      'x-piqae-dashboard': '1'
    }
  });
  if (!response.ok) error(409, 'The current Piqae workspace could not be verified');
  const workspace = (await response.json()) as { id?: unknown };
  if (typeof workspace.id !== 'string' || workspace.id === '') {
    error(409, 'The current Piqae workspace identity is invalid');
  }

  const stripe = stripeClient();
  const customer = await findStripeCustomer(stripe, workspace.id);
  if (!customer) error(404, 'No managed Stripe customer exists for this workspace');
  const session = await stripe.billingPortal.sessions.create({
    customer: customer.id,
    return_url: `${event.url.origin}/dashboard/settings#billing`
  });
  return json(
    { url: session.url },
    { headers: { 'cache-control': 'no-store, private' } }
  );
};
