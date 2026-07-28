import { authKit } from '@workos/authkit-sveltekit';
import { error, redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode, workosConfigured } from '$lib/server/auth-config';

export const GET: RequestHandler = async ({ url }) => {
  const returnTo = url.searchParams.get('return_to') ?? '/dashboard';
  if (authMode === 'demo') redirect(303, returnTo);
  if (authMode !== 'workos' || !workosConfigured) {
    error(503, 'Hosted authentication is not configured for this deployment');
  }
  const signInUrl = await authKit.getSignInUrl({ returnTo });
  redirect(302, signInUrl);
};
