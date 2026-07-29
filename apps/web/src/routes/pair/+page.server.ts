import { authKit } from '@workos/authkit-sveltekit';
import { fail, redirect } from '@sveltejs/kit';
import type { RequestEvent } from '@sveltejs/kit';
import type { Actions, PageServerLoad } from './$types';
import { authMode } from '$lib/server/auth-config';
import {
  dashboardMode,
  dashboardSdk,
  preventSecretCaching,
  presentDashboardError
} from '$lib/server/dashboard-data';

const authorizationIdPattern = /^dva_[0-9A-HJKMNP-TV-Z]{26}$/;

async function requirePairingOperator(event: RequestEvent): Promise<void> {
  if (authMode !== 'workos') return;
  const user = await authKit.getUser(event);
  if (!user) {
    const returnTo = `${event.url.pathname}${event.url.search}`;
    const signInUrl = await authKit.getSignInUrl({ returnTo });
    redirect(302, signInUrl);
  }
}

function authorizationId(value: FormDataEntryValue | string | null): string | null {
  const normalized = String(value ?? '').trim();
  return authorizationIdPattern.test(normalized) ? normalized : null;
}

export const load: PageServerLoad = async (event) => {
  preventSecretCaching(event);
  await requirePairingOperator(event);
  const id = authorizationId(event.url.searchParams.get('authorization_id'));
  if (!id) return { authorization: null, loadError: null };
  if (dashboardMode() !== 'live') {
    return {
      authorization: null,
      loadError: 'Pairing is unavailable while the dashboard uses demo data.'
    };
  }
  try {
    return {
      authorization: await dashboardSdk(event).pairing.review(id),
      loadError: null
    };
  } catch (error) {
    return { authorization: null, loadError: presentDashboardError(error).message };
  }
};

export const actions: Actions = {
  approve: async (event) => decide(event, 'approve'),
  deny: async (event) => decide(event, 'deny')
};

async function decide(
  event: Parameters<NonNullable<Actions['approve']>>[0],
  decision: 'approve' | 'deny'
) {
  preventSecretCaching(event);
  await requirePairingOperator(event);
  if (dashboardMode() !== 'live') {
    return fail(400, { decision, error: 'Pairing is disabled while demo data is active.' });
  }
  const form = await event.request.formData();
  const id = authorizationId(form.get('authorization_id'));
  const userCode = String(form.get('user_code') ?? '')
    .trim()
    .toUpperCase();
  if (!id || !/^[2-9A-HJ-NP-Z]{4}-[2-9A-HJ-NP-Z]{4}$/.test(userCode)) {
    return fail(400, {
      decision,
      error: 'Enter the eight-character code displayed by the Piqae node.'
    });
  }
  try {
    const result =
      decision === 'approve'
        ? await dashboardSdk(event).pairing.approve(id, userCode)
        : await dashboardSdk(event).pairing.deny(id, userCode);
    return { decision, state: result.state, error: null };
  } catch (error) {
    return fail(400, { decision, error: presentDashboardError(error).message });
  }
}
