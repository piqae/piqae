import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types';

export const POST: RequestHandler = async ({ locals }) => {
  if (!locals.auth?.user) return json({ error: 'unauthenticated' }, { status: 401 });
  // A five-minute Spool API token must be minted by the Rust control plane.
  return json({ error: 'token_exchange_unavailable' }, { status: 501 });
};
