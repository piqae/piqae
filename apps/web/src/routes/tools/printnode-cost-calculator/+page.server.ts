import type { PageServerLoad } from './$types';
import { loadPrintNodePricingSnapshot } from '$lib/server/marketing-content';
import { pricingCatalog } from '$lib/server/pricing';

export const load: PageServerLoad = async ({ fetch }) => ({
  printNodeSnapshot: await loadPrintNodePricingSnapshot(fetch),
  piqaePricing: pricingCatalog()
});
