import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

// Legacy compatibility route. Agents were renamed to nodes.
export const load: PageServerLoad = ({ params }) => {
  redirect(308, `/dashboard?node=${encodeURIComponent(params.id)}`);
};
