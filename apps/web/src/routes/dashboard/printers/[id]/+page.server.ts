import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

// Detail is a drawer on /dashboard, addressed by query string.
export const load: PageServerLoad = ({ params }) => {
  redirect(308, `/dashboard?printer=${encodeURIComponent(params.id)}`);
};
