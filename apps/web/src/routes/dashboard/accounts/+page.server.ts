import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

// Folded into the operations surface; only rendered where accounts are enabled.
export const load: PageServerLoad = () => {
  redirect(308, '/dashboard?view=customers');
};
