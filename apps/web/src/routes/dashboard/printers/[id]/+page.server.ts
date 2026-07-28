import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import { dashboardSource, presentDashboardError } from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => {
  try {
    const { api } = dashboardSource(event);
    const [printers, agents, jobs] = await Promise.all([api.printers(), api.agents(), api.jobs()]);
    const printer = printers.data.find((candidate) => candidate.id === event.params.id);
    if (!printer) error(404, 'Printer not found');
    return {
      printer,
      agent: agents.data.find((candidate) => candidate.id === printer.agentId) ?? null,
      jobs: jobs.data.filter((job) => job.printerId === printer.id),
      dataError: null
    };
  } catch (caught) {
    if (caught && typeof caught === 'object' && 'status' in caught && caught.status === 404) throw caught;
    return { printer: null, agent: null, jobs: [], dataError: presentDashboardError(caught) };
  }
};
