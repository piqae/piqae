import { authKit } from '@workos/authkit-sveltekit';
import { redirect } from '@sveltejs/kit';
import type { LayoutServerLoad } from './$types';
import { authMode } from '$lib/server/auth-config';

export const load: LayoutServerLoad = async (event) => {
  if (authMode === 'demo') {
    return {
      viewer: {
        id: 'usr_demo',
        email: 'developer@spool.local',
        name: 'Spool Developer',
        organizationId: 'org_demo'
      }
    };
  }

  if (authMode === 'workos') {
    const user = await authKit.getUser(event);
    if (!user) {
      const signInUrl = await authKit.getSignInUrl({ returnTo: event.url.pathname });
      redirect(302, signInUrl);
    }
    return {
      viewer: {
        id: user.id,
        email: user.email,
        name: `${user.firstName ?? ''} ${user.lastName ?? ''}`.trim() || null,
        organizationId: event.locals.auth?.organizationId ?? null
      }
    };
  }

  // Local/self-host mode remains independent of WorkOS. The control plane or
  // reverse proxy can supply its own session boundary.
  return { viewer: null };
};
