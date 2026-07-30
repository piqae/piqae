import { authKit } from '@workos/authkit-sveltekit';
import { error, redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode } from '$lib/server/auth-config';
import { listUserMemberships } from '$lib/server/workos-admin';

export const POST: RequestHandler = async (event) => {
  if (authMode !== 'workos') error(404, 'Session refresh is unavailable');
  const user = await authKit.getUser(event);
  if (!user) redirect(303, '/login');
  const organizationId = event.locals.auth?.organizationId;
  if (!organizationId) redirect(303, '/onboarding');
  const memberships = await listUserMemberships(user.id);
  if (
    !memberships.some(
      (membership) =>
        membership.organizationId === organizationId && membership.status === 'active'
    )
  ) {
    error(403, 'Your workspace access has been removed');
  }
  const response = await authKit.switchOrganization(event, { organizationId });
  response.headers.set('Location', '/dashboard/settings/team');
  return response;
};
