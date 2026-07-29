import { authKitHandle, configureAuthKit } from '@workos/authkit-sveltekit';
import type { Handle } from '@sveltejs/kit';
import { authMode, workosConfig } from '$lib/server/auth-config';
import { localSessionToken } from '$lib/server/local-owner-auth';

const hostedHandle: Handle | null = workosConfig
  ? (() => {
      configureAuthKit(workosConfig);
      return authKitHandle({
        debug: false,
        onError: (error) => console.error('Hosted authentication failed', error)
      });
    })()
  : null;

export const handle: Handle = async ({ event, resolve }) => {
  event.locals.authMode = authMode;
  if (authMode === 'local') {
    event.locals.localSessionToken = localSessionToken(event.cookies) ?? undefined;
  }
  if (authMode === 'workos' && hostedHandle) return hostedHandle({ event, resolve });
  return resolve(event);
};
