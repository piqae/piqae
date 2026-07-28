import { error } from '@sveltejs/kit';
import { agents, jobs, printers } from '$lib/demo-data';

export function load({ params }) {
  const printer = printers.find((candidate) => candidate.id === params.id);
  if (!printer) error(404, 'Printer not found');
  return {
    printer,
    agent: agents.find((candidate) => candidate.id === printer.agentId),
    jobs: jobs.filter((job) => job.printerId === printer.id)
  };
}
