import type { PageServerLoad } from './$types';
import { dashboardSource, presentDashboardError } from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => {
  try {
    const agents = await dashboardSource(event).api.agents();
    return { agents: agents.data, dataError: null };
  } catch (error) {
    return { agents: [], dataError: presentDashboardError(error) };
  }
};
