import { authKitHandle, configureAuthKit } from '@workos/authkit-sveltekit';
import type { Handle } from '@sveltejs/kit';
import { authMode, workosConfig } from '$lib/server/auth-config';

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
  if (authMode === 'workos' && hostedHandle) return hostedHandle({ event, resolve });
  return resolve(event);
};
