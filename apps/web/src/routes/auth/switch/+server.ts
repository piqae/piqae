import { authKit } from '@workos/authkit-sveltekit';
import { error, redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode } from '$lib/server/auth-config';
import { isSameOriginRequest } from '$lib/server/same-origin';
import { safeReturnTo } from '$lib/server/safe-return-to';
import { listUserMemberships } from '$lib/server/workos-admin';

export const POST: RequestHandler = async (event) => {
  if (authMode !== 'workos') error(404, 'Workspace switching is unavailable');
  if (!isSameOriginRequest(event.request, event.url)) {
    error(403, 'Cross-origin workspace switching is denied');
  }
  const user = await authKit.getUser(event);
  if (!user) redirect(303, '/login');
  const form = await event.request.formData();
  const organizationId = form.get('organization_id');
  const requestedReturnTo = form.get('return_to');
  const returnTo = safeReturnTo(
    typeof requestedReturnTo === 'string' ? requestedReturnTo : null
  );
  if (typeof organizationId !== 'string' || !organizationId) {
    error(400, 'A workspace is required');
  }
  const memberships = await listUserMemberships(user.id);
  if (
    !memberships.some(
      (membership) =>
        membership.organizationId === organizationId && membership.status === 'active'
    )
  ) {
    error(403, 'You do not have access to that workspace');
  }
  const response = await authKit.switchOrganization(event, { organizationId });
  response.headers.set('Location', returnTo);
  return response;
};
