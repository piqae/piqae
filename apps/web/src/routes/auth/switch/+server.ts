import { authKit } from '@workos/authkit-sveltekit';
import { error, redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode } from '$lib/server/auth-config';
import { listUserMemberships } from '$lib/server/workos-admin';

export const POST: RequestHandler = async (event) => {
  if (authMode !== 'workos') error(404, 'Workspace switching is unavailable');
  const user = await authKit.getUser(event);
  if (!user) redirect(303, '/login');
  const form = await event.request.formData();
  const organizationId = form.get('organization_id');
  const returnTo = safeReturnTo(form.get('return_to'));
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

function safeReturnTo(value: FormDataEntryValue | null): string {
  return typeof value === 'string' && value.startsWith('/') && !value.startsWith('//')
    ? value
    : '/dashboard';
}
