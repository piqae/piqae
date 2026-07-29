import type { PageServerLoad } from './$types';
import { loadPricingProse } from '$lib/server/marketing-content';
import { pricingCatalog } from '$lib/server/pricing';

export const load: PageServerLoad = async ({ fetch }) => ({
  pricing: pricingCatalog(await loadPricingProse(fetch))
});
