import type { PageServerLoad } from './$types';
import {
  dashboardSource,
  preventSecretCaching,
  presentDashboardError
} from '$lib/server/dashboard-data';

export const load: PageServerLoad = async (event) => {
  preventSecretCaching(event);
  const { meta } = await event.parent();
  if (!meta.platform.accounts) {
    return { available: false, account: null, dataError: null };
  }

  try {
    const account = await dashboardSource(event).api.account(event.params.external_id);
    return { available: true, account, dataError: null };
  } catch (error) {
    return {
      available: true,
      account: null,
      dataError: presentDashboardError(error)
    };
  }
};
