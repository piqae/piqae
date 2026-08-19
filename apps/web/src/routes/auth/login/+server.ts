import { authKit } from '@workos/authkit-sveltekit';
import { error, redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode, workosConfigured } from '$lib/server/auth-config';
import { safeReturnTo } from '$lib/server/safe-return-to';

export const GET: RequestHandler = async ({ url }) => {
  const returnTo = safeReturnTo(url.searchParams.get('return_to'));
  if (authMode === 'demo') redirect(303, returnTo);
  if (authMode === 'local') redirect(303, `/login?return_to=${encodeURIComponent(returnTo)}`);
  if (authMode !== 'workos' || !workosConfigured) {
    error(503, 'Hosted authentication is not configured for this deployment');
  }
  if (url.searchParams.get('hosted') !== '1') {
    redirect(303, `/login?return_to=${encodeURIComponent(returnTo)}`);
  }
  const signInUrl = await authKit.getSignInUrl({ returnTo });
  redirect(302, signInUrl);
};
