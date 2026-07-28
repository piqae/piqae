import { error } from '@sveltejs/kit';
import { jobs } from '$lib/demo-data';

export function load({ params }) {
  const job = jobs.find((candidate) => candidate.id === params.id);
  if (!job) error(404, 'Job not found');
  return { job };
}
