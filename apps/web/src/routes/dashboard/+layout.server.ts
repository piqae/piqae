import { authKit } from '@workos/authkit-sveltekit';
import { redirect } from '@sveltejs/kit';
import type { LayoutServerLoad } from './$types';
import { authMode } from '$lib/server/auth-config';
import { dashboardMeta, dashboardMode } from '$lib/server/dashboard-data';

export const load: LayoutServerLoad = async (event) => {
  const mode = dashboardMode();
  const meta = await dashboardMeta(event);
  if (authMode === 'demo') {
    return {
      dashboardMode: mode,
      meta,
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
      dashboardMode: mode,
      meta,
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
  return { dashboardMode: mode, meta, viewer: null };
};
