import type { PageServerLoad } from './$types';
import { pricingCatalog } from '$lib/server/pricing';

export const load: PageServerLoad = () => ({
  pricing: pricingCatalog()
});
