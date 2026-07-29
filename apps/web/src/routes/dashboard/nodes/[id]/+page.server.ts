import { error } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import { dashboardSource, presentDashboardError } from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => {
  try {
    const { api } = dashboardSource(event);
    const [agents, printers] = await Promise.all([api.agents(), api.printers()]);
    const node = agents.data.find((candidate) => candidate.id === event.params.id);
    if (!node) error(404, 'Node not found');
    return {
      node,
      printers: printers.data.filter((printer) => printer.agentId === node.id),
      dataError: null
    };
  } catch (caught) {
    if (caught && typeof caught === 'object' && 'status' in caught && caught.status === 404) throw caught;
    return { node: null, printers: [], dataError: presentDashboardError(caught) };
  }
};
