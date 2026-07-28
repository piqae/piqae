import { error } from '@sveltejs/kit';
import { agents, printers } from '$lib/demo-data';

export function load({ params }) {
  const agent = agents.find((candidate) => candidate.id === params.id);
  if (!agent) error(404, 'Agent not found');
  return { agent, printers: printers.filter((printer) => printer.agentId === agent.id) };
}
