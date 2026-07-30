import { authKit } from '@workos/authkit-sveltekit';
import { error, redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode, workosConfigured } from '$lib/server/auth-config';

export const GET: RequestHandler = async ({ url }) => {
  if (authMode !== 'workos' || !workosConfigured) {
    error(503, 'Hosted authentication is not configured for this deployment');
  }
  const requested = url.searchParams.get('return_to');
  const returnTo =
    requested?.startsWith('/') && !requested.startsWith('//') ? requested : '/dashboard';
  const signUpUrl = await authKit.getSignUpUrl({ returnTo });
  redirect(302, signUpUrl);
};
