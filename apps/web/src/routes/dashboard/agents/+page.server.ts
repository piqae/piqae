import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
export { actions } from '../nodes/+page.server';

export const load: PageServerLoad = ({ url }) => {
  redirect(308, `/dashboard/nodes${url.search}`);
};
