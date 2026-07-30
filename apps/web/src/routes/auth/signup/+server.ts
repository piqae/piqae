import { authKit } from '@workos/authkit-sveltekit';
import { error, redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode, workosConfigured } from '$lib/server/auth-config';
import { safeReturnTo } from '$lib/server/safe-return-to';

export const GET: RequestHandler = async ({ url }) => {
  if (authMode !== 'workos' || !workosConfigured) {
    error(503, 'Hosted authentication is not configured for this deployment');
  }
  const requested = url.searchParams.get('return_to');
  const returnTo = safeReturnTo(requested);
  const signUpUrl = await authKit.getSignUpUrl({ returnTo });
  redirect(302, signUpUrl);
};
