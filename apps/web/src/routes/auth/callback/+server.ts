import { authKit } from '@workos/authkit-sveltekit';
import { error } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode, workosConfigured } from '$lib/server/auth-config';

export const GET: RequestHandler = async (event) => {
  if (authMode !== 'workos' || !workosConfigured) {
    error(503, 'Hosted authentication is not configured for this deployment');
  }
  return authKit.handleCallback()(event);
};
