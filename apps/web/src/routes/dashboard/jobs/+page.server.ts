import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

// Folded into the single operations surface at /dashboard?view=jobs.
export const load: PageServerLoad = () => {
  redirect(308, '/dashboard?view=jobs');
};
