import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import { dashboardSource, presentDashboardError } from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => {
  try {
    const { api } = dashboardSource(event);
    const [job, jobEvents, printers, agents] = await Promise.all([
      api.job(event.params.id),
      api.jobEvents(event.params.id),
      api.printers(),
      api.agents()
    ]);
    if (!job) error(404, 'Job not found');
    return {
      job,
      jobEvents: jobEvents.data,
      printer: printers.data.find((candidate) => candidate.id === job.printerId) ?? null,
      agent: agents.data.find((candidate) => candidate.id === job.agentId) ?? null,
      dataError: null
    };
  } catch (caught) {
    if (caught && typeof caught === 'object' && 'status' in caught && caught.status === 404) throw caught;
    return { job: null, jobEvents: [], printer: null, agent: null, dataError: presentDashboardError(caught) };
  }
};
