import { fail } from '@sveltejs/kit';
import type { ApiKeyScope } from '@spool/sdk';
import type { Actions, PageServerLoad } from './$types';
import {
  dashboardMode,
  dashboardSdk,
  dashboardSource,
  preventSecretCaching,
  presentDashboardError
} from '$lib/server/dashboard-data';

const allowedScopes = new Set<ApiKeyScope>([
  'api_keys_read',
  'api_keys_write',
  'agents_read',
  'agents_write',
  'printers_read',
  'printers_write',
  'jobs_read',
  'jobs_write',
  'webhooks_read',
  'webhooks_write',
  'usage_read',
  'audit_read'
]);

export const load: PageServerLoad = async (event) => {
  preventSecretCaching(event);
  try {
    const apiKeys = await dashboardSource(event).api.apiKeys();
    return { apiKeys: apiKeys.data, dataError: null };
  } catch (error) {
    return { apiKeys: [], dataError: presentDashboardError(error) };
  }
};

export const actions: Actions = {
  createApiKey: async (event) => {
    preventSecretCaching(event);
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'createApiKey',
        error: { message: 'API-key mutations are disabled while demo data is active.' }
      });
    }

    const data = await event.request.formData();
    const name = String(data.get('name') ?? '').trim();
    const requestedScopes = [...new Set(data.getAll('scopes').map(String))];
    const expiresAtValue = String(data.get('expires_at') ?? '').trim();
    if (name.length < 2 || name.length > 120) {
      return fail(400, {
        mutation: 'createApiKey',
        error: { message: 'Key name must be between 2 and 120 characters.' }
      });
    }
    if (
      requestedScopes.length === 0 ||
      requestedScopes.some((scope) => !allowedScopes.has(scope as ApiKeyScope))
    ) {
      return fail(400, {
        mutation: 'createApiKey',
        error: { message: 'Select one or more supported API scopes.' }
      });
    }

    let expiresAt: string | null = null;
    if (expiresAtValue) {
      const parsed = new Date(expiresAtValue);
      if (Number.isNaN(parsed.valueOf()) || parsed <= new Date()) {
        return fail(400, {
          mutation: 'createApiKey',
          error: { message: 'Expiry must be a valid future date.' }
        });
      }
      expiresAt = parsed.toISOString();
    }

    try {
      const apiKey = await dashboardSdk(event).apiKeys.create({
        name,
        scopes: requestedScopes as ApiKeyScope[],
        expires_at: expiresAt
      });
      return {
        mutation: 'createApiKey',
        apiKey: {
          id: apiKey.id,
          name: apiKey.name,
          prefix: apiKey.lookup_prefix,
          secret: apiKey.secret
        }
      };
    } catch (error) {
      return fail(502, {
        mutation: 'createApiKey',
        error: { message: presentDashboardError(error).message }
      });
    }
  },
  revokeApiKey: async (event) => {
    if (dashboardMode() !== 'live') {
      return fail(400, {
        mutation: 'revokeApiKey',
        error: { message: 'API-key mutations are disabled while demo data is active.' }
      });
    }
    const data = await event.request.formData();
    const apiKeyId = String(data.get('api_key_id') ?? '').trim();
    if (!apiKeyId) {
      return fail(400, {
        mutation: 'revokeApiKey',
        error: { message: 'API key ID is required.' }
      });
    }
    try {
      await dashboardSdk(event).apiKeys.revoke(apiKeyId);
      return { mutation: 'revokeApiKey', revokedApiKeyId: apiKeyId };
    } catch (error) {
      return fail(502, {
        mutation: 'revokeApiKey',
        error: { message: presentDashboardError(error).message }
      });
    }
  }
};
