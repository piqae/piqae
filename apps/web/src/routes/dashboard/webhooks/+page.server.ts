import type { PageServerLoad } from './$types';
import { dashboardSource, presentDashboardError } from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => {
  try {
    const webhooks = await dashboardSource(event).api.webhooks();
    return { webhooks: webhooks.data, dataError: null };
  } catch (error) {
    return { webhooks: [], dataError: presentDashboardError(error) };
  }
};
