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
    return { available: false, accounts: [], dataError: null };
  }

  try {
    const accounts = await dashboardSource(event).api.accounts();
    return { available: true, accounts: accounts.data, dataError: null };
  } catch (error) {
    return {
      available: true,
      accounts: [],
      dataError: presentDashboardError(error)
    };
  }
};
