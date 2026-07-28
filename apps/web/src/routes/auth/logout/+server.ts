import { authKit } from '@workos/authkit-sveltekit';
import { redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode } from '$lib/server/auth-config';

export const GET: RequestHandler = async (event) => {
  const returnTo = event.url.searchParams.get('return_to') ?? '/login';
  if (authMode !== 'workos') redirect(303, returnTo);
  return authKit.signOut(event);
};
