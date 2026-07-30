import { randomUUID } from 'node:crypto';
import { authKit } from '@workos/authkit-sveltekit';
import { redirect } from '@sveltejs/kit';
import type { PageServerLoad } from './$types';
import { authMode } from '$lib/server/auth-config';
import { listUserMemberships } from '$lib/server/workos-admin';

export const load: PageServerLoad = async (event) => {
  if (authMode !== 'workos') redirect(303, '/dashboard');
  const user = await authKit.getUser(event);
  if (!user) {
    const url = await authKit.getSignInUrl({ returnTo: '/onboarding' });
    redirect(302, url);
  }
  return {
    user: {
      email: user.email,
      name: `${user.firstName ?? ''} ${user.lastName ?? ''}`.trim() || null
    },
    memberships: await listUserMemberships(user.id),
    workspaceToken: randomUUID()
  };
};
