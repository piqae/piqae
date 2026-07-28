import type { PageServerLoad } from './$types';
import { dashboardSource, presentDashboardError } from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => {
  try {
    const { api } = dashboardSource(event);
    const [jobs, printers, agents] = await Promise.all([api.jobs(), api.printers(), api.agents()]);
    return { jobs: jobs.data, printers: printers.data, agents: agents.data, dataError: null };
  } catch (error) {
    return { jobs: [], printers: [], agents: [], dataError: presentDashboardError(error) };
  }
};
