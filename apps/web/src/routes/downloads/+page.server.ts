import type { PageServerLoad } from './$types';
import { dashboardMeta } from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => ({ meta: await dashboardMeta(event) });
