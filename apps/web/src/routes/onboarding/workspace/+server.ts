import { authKit } from '@workos/authkit-sveltekit';
import { error, redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode } from '$lib/server/auth-config';
import {
  createOrganization,
  ensureOrganizationMembership
} from '$lib/server/workos-admin';

export const POST: RequestHandler = async (event) => {
  if (authMode !== 'workos') error(404, 'Workspace creation is unavailable');
  const user = await authKit.getUser(event);
  if (!user) redirect(303, '/login');
  const form = await event.request.formData();
  const rawName = form.get('name');
  const token = form.get('workspace_token');
  const name = typeof rawName === 'string' ? rawName.trim() : '';
  if (name.length < 2 || name.length > 100) {
    error(400, 'Workspace names must be between 2 and 100 characters');
  }
  if (
    typeof token !== 'string' ||
    !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i.test(
      token
    )
  ) {
    error(400, 'The workspace request expired; reload and try again');
  }
  const recoveryKey = `piqae:${user.id}:${token}`;
  const organization = await createOrganization(name, recoveryKey);
  await ensureOrganizationMembership(
    organization.id,
    user.id,
    'owner',
    `${recoveryKey}:owner-membership`
  );
  const response = await authKit.switchOrganization(event, {
    organizationId: organization.id
  });
  response.headers.set('Location', '/dashboard');
  return response;
};
