import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';

// Folded into /dashboard/settings#api-keys. The query string is preserved so
// callbacks that carry state (Stripe's ?checkout=success) still reach the page.
export const load: PageServerLoad = ({ url }) => {
  redirect(308, `/dashboard/settings${url.search}#api-keys`);
};
