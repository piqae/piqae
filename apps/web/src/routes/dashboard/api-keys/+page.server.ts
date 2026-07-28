import type { PageServerLoad } from './$types';
import { dashboardSource, presentDashboardError } from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => {
  try {
    const apiKeys = await dashboardSource(event).api.apiKeys();
    return { apiKeys: apiKeys.data, dataError: null };
  } catch (error) {
    return { apiKeys: [], dataError: presentDashboardError(error) };
  }
};
