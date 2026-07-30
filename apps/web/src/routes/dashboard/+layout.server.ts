import { authKit } from '@workos/authkit-sveltekit';
import { redirect } from '@sveltejs/kit';
import type { LayoutServerLoad } from './$types';
import { authMode } from '$lib/server/auth-config';
import { dashboardMeta, dashboardMode } from '$lib/server/dashboard-data';
import { currentLocalIdentity } from '$lib/server/local-owner-auth';

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
        name: 'Piqae Developer',
        organizationId: 'org_demo',
        role: 'owner'
      }
    };
  }

  if (authMode === 'workos') {
    const user = await authKit.getUser(event);
    if (!user) {
      const signInUrl = await authKit.getSignInUrl({ returnTo: event.url.pathname });
      redirect(302, signInUrl);
    }
    if (!event.locals.auth?.organizationId) {
      redirect(303, '/onboarding');
    }
    return {
      dashboardMode: mode,
      meta,
      viewer: {
        id: user.id,
        email: user.email,
        name: `${user.firstName ?? ''} ${user.lastName ?? ''}`.trim() || null,
        organizationId: event.locals.auth.organizationId,
        role: event.locals.auth?.role ?? null
      }
    };
  }

  const token = event.locals.localSessionToken;
  if (!token) redirect(303, `/login?return_to=${encodeURIComponent(event.url.pathname)}`);
  const user = await currentLocalIdentity(event, token);
  if (!user) redirect(303, `/login?return_to=${encodeURIComponent(event.url.pathname)}`);
  return {
    dashboardMode: mode,
    meta,
    viewer: {
      id: user.id,
      email: user.email,
      name: user.name,
      organizationId: user.workspaceId,
      role: 'owner'
    }
  };
};
