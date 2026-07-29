import { fail, redirect } from '@sveltejs/kit';
import type { Actions, PageServerLoad } from './$types';
import { authMode } from '$lib/server/auth-config';
import { currentLocalIdentity, exchangeLocalOwnerCredential } from '$lib/server/local-owner-auth';

function safeReturnTo(value: FormDataEntryValue | string | null): string {
  return typeof value === 'string' && value.startsWith('/') && !value.startsWith('//')
    ? value
    : '/dashboard';
}

export const load: PageServerLoad = async (event) => {
  const returnTo = safeReturnTo(event.url.searchParams.get('return_to'));
  if (authMode === 'demo') redirect(303, returnTo);
  if (authMode === 'local' && (await currentLocalIdentity(event))) redirect(303, returnTo);
  return { authMode, returnTo };
};

export const actions: Actions = {
  default: async (event) => {
    if (authMode !== 'local') redirect(303, '/auth/login');
    const origin = event.request.headers.get('origin');
    if (!origin || origin !== event.url.origin) return fail(403, { invalid: true });
    const data = await event.request.formData();
    const credential = data.get('credential');
    const returnTo = safeReturnTo(data.get('return_to'));
    if (typeof credential !== 'string' || credential.length < 40 || credential.length > 512) {
      return fail(400, { invalid: true });
    }
    try {
      await exchangeLocalOwnerCredential(event, credential);
    } catch {
      return fail(401, { invalid: true });
    }
    redirect(303, returnTo);
  }
};
