import { fail } from '@sveltejs/kit';
import type { Actions, PageServerLoad } from './$types';
import {
  dashboardMode,
  dashboardSdk,
  dashboardSource,
  preventSecretCaching,
  presentDashboardError
} from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => {
  try {
    const nodes = await dashboardSource(event).api.agents();
    return { nodes: nodes.data, dataError: null };
  } catch (error) {
    return { nodes: [], dataError: presentDashboardError(error) };
  }
};

export const actions: Actions = {
  createEnrolment: async (event) => {
    preventSecretCaching(event);
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'createEnrolment',
        error: { message: 'Node enrolment is disabled while demo data is active.' }
      });
    }
    const data = await event.request.formData();
    const name = String(data.get('name') ?? '').trim();
    const expiresInSeconds = Number(data.get('expires_in_seconds') ?? 600);
    if (name.length < 2 || name.length > 120) {
      return fail(400, {
        mutation: 'createEnrolment',
        error: { message: 'Node name must be between 2 and 120 characters.' }
      });
    }
    if (!Number.isInteger(expiresInSeconds) || expiresInSeconds < 60 || expiresInSeconds > 3600) {
      return fail(400, {
        mutation: 'createEnrolment',
        error: { message: 'Expiry must be between 60 and 3,600 seconds.' }
      });
    }
    try {
      const enrolment = await dashboardSdk(event).agents.createEnrolment({
        name,
        expires_in_seconds: expiresInSeconds
      });
      return {
        mutation: 'createEnrolment',
        enrolment: {
          id: enrolment.id,
          token: enrolment.token,
          expiresAt: enrolment.expires_at
        }
      };
    } catch (error) {
      return fail(502, {
        mutation: 'createEnrolment',
        error: { message: presentDashboardError(error).message }
      });
    }
  }
};
