import type { PageServerLoad } from './$types';
import { dashboardSource, presentDashboardError } from '$lib/server/dashboard-data';
import type { DashboardOverview } from '$lib/view-types';

const emptyOverview: DashboardOverview = {
  agents: { total: 0, online: 0, degraded: 0 },
  printers: { total: 0, online: 0, attention: 0 },
  jobs: { today: 0, active: 0, failed: 0, uncertain: 0 },
  pickupLatencyP95Ms: 0
};

export const load: PageServerLoad = async (event) => {
  try {
    const { api } = dashboardSource(event);
    const [overview, jobs, printers] = await Promise.all([
      api.overview(),
      api.jobs(),
      api.printers()
    ]);
    return { overview, jobs: jobs.data.slice(0, 5), printers: printers.data, dataError: null };
  } catch (error) {
    return { overview: emptyOverview, jobs: [], printers: [], dataError: presentDashboardError(error) };
  }
};
