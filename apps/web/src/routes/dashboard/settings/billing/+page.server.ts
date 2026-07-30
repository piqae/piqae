import type { PageServerLoad } from './$types';
import type { MarketingAttribution } from '$lib/marketing/attribution';
import {
  canManageHostedBilling,
  checkoutAllowed,
  parseBillingSummary,
  parseUsageSummary,
  stripePortalAvailable
} from '$lib/server/billing';
import {
  dashboardConnection,
  preventSecretCaching,
  presentDashboardError
} from '$lib/server/dashboard-data';
import { pricingCatalog } from '$lib/server/pricing';

export const load: PageServerLoad = async (event) => {
  preventSecretCaching(event);
  const { cookies, url } = event;
  let attribution: MarketingAttribution | null = null;
  const stored = cookies.get('piqae_attribution');
  if (stored) {
    try {
      attribution = JSON.parse(Buffer.from(stored, 'base64url').toString('utf8')) as MarketingAttribution;
    } catch {
      attribution = null;
    }
  }
  const base = {
    pricing: pricingCatalog(),
    selectedInterval:
      attribution?.interval === 'annual' || attribution?.interval === 'monthly'
        ? attribution.interval
        : 'monthly',
    checkoutState: url.searchParams.get('checkout'),
    checkoutAvailable: {
      monthly: checkoutAllowed('pro', 'monthly'),
      annual: checkoutAllowed('pro', 'annual')
    },
    portalAvailable: stripePortalAvailable()
  };
  const { meta } = await event.parent();
  const canManageBilling = canManageHostedBilling(event.locals.auth?.role);
  if (!meta.billing.enabled) {
    return {
      ...base,
      available: false,
      canManageBilling,
      summary: null,
      workspaceUsage: null,
      dataError: null
    };
  }
  try {
    const { baseUrl, bearerToken } = dashboardConnection(event);
    const headers = {
      accept: 'application/json',
      authorization: `Bearer ${bearerToken}`,
      'x-piqae-dashboard': '1'
    };
    const [summaryResponse, usageResponse] = await Promise.all([
      event.fetch(`${baseUrl.replace(/\/$/, '')}/v1/billing/summary`, { headers }),
      event.fetch(`${baseUrl.replace(/\/$/, '')}/v1/usage`, { headers })
    ]);
    if (!summaryResponse.ok || !usageResponse.ok) {
      throw new Error(
        `Piqae billing request failed with HTTP ${summaryResponse.status}/${usageResponse.status}.`
      );
    }
    return {
      ...base,
      available: true,
      canManageBilling,
      summary: parseBillingSummary(await summaryResponse.json()),
      workspaceUsage: parseUsageSummary(await usageResponse.json()),
      dataError: null
    };
  } catch (error) {
    return {
      ...base,
      available: true,
      canManageBilling,
      summary: null,
      workspaceUsage: null,
      dataError: presentDashboardError(error)
    };
  }
};
