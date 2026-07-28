import { error } from '@sveltejs/kit';
import { docBySlug } from '$lib/docs-content';

export function load({ params }) {
  const doc = docBySlug.get(params.slug);
  if (!doc) error(404, 'Documentation page not found');
  return { doc };
}
