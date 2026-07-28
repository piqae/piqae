import type { PageServerLoad } from './$types';
import { dashboardSource, presentDashboardError } from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => {
  try {
    const { api } = dashboardSource(event);
    const [printers, agents] = await Promise.all([api.printers(), api.agents()]);
    return { printers: printers.data, agents: agents.data, dataError: null };
  } catch (error) {
    return { printers: [], agents: [], dataError: presentDashboardError(error) };
  }
};
