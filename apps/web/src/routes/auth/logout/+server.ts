import { authKit } from '@workos/authkit-sveltekit';
import { redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode } from '$lib/server/auth-config';
import { revokeLocalSession } from '$lib/server/local-owner-auth';

export const GET: RequestHandler = async (event) => {
  const returnTo = event.url.searchParams.get('return_to') ?? '/login';
  if (authMode === 'local') {
    await revokeLocalSession(event);
    redirect(303, returnTo);
  }
  if (authMode !== 'workos') redirect(303, returnTo);
  return authKit.signOut(event);
};
