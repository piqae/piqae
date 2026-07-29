import { authKit } from '@workos/authkit-sveltekit';
import { json } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { authMode } from '$lib/server/auth-config';
import { currentLocalIdentity, ensureLocalSession } from '$lib/server/local-owner-auth';

export const GET: RequestHandler = async (event) => {
  if (authMode === 'demo') {
    return json({
      id: 'usr_demo',
      email: 'developer@spool.local',
      name: 'Spool Developer',
      workspaceId: 'wrk_demo',
      roles: ['owner']
    });
  }
  if (authMode === 'local') {
    const token = await ensureLocalSession(event);
    const user = token ? await currentLocalIdentity(event, token) : null;
    if (!user) return json({ error: 'unauthenticated' }, { status: 401 });
    return json({
      id: user.id,
      email: user.email,
      name: user.name,
      workspaceId: user.workspaceId,
      roles: user.roles
    });
  }
  if (authMode !== 'workos') return json({ error: 'unauthenticated' }, { status: 401 });
  const user = await authKit.getUser(event);
  if (!user) return json({ error: 'unauthenticated' }, { status: 401 });
  return json({
    id: user.id,
    email: user.email,
    name: `${user.firstName ?? ''} ${user.lastName ?? ''}`.trim() || null,
    workspaceId: event.locals.auth?.organizationId ?? null,
    roles: event.locals.auth?.role ? [event.locals.auth.role] : []
  });
};
