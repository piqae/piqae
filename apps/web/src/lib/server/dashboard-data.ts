import { env as privateEnv } from '$env/dynamic/private';
import { env as publicEnv } from '$env/dynamic/public';
import type { RequestEvent } from '@sveltejs/kit';
import { SpoolClient, SpoolError } from '@spool/sdk';
import { createLiveApi, mockApi, parseDashboardMeta, type DashboardApi } from '$lib/api';
import type { DashboardMeta } from '$lib/view-types';

export type DashboardMode = 'live' | 'demo';

export interface DashboardSource {
  api: DashboardApi;
  mode: DashboardMode;
}

export interface DashboardConnection {
  baseUrl: string;
  bearerToken: string;
}

export interface DashboardLoadError {
  title: string;
  message: string;
  code: string;
  requestId: string | null;
  retryable: boolean;
}

export function preventSecretCaching(event: Pick<RequestEvent, 'setHeaders'>): void {
  event.setHeaders({ 'cache-control': 'no-store, private' });
}

function configuredMode(): DashboardMode {
  const value = publicEnv.PUBLIC_SPOOL_DASHBOARD_MODE;
  if (value === 'demo') return 'demo';
  if (value === undefined || value === '' || value === 'live') return 'live';
  throw new Error('PUBLIC_SPOOL_DASHBOARD_MODE must be live or demo');
}

export function dashboardConnection(
  event: Pick<RequestEvent, 'url' | 'locals'>
): DashboardConnection {
  const baseUrl = publicEnv.PUBLIC_SPOOL_API_URL || event.url.origin;
  let bearerToken: string | undefined;

  if (event.locals.authMode === 'workos') {
    bearerToken = event.locals.auth?.accessToken;
    if (!bearerToken) {
      throw new Error('The verified hosted session does not contain an OIDC access token.');
    }
  } else {
    bearerToken = event.locals.localSessionToken ?? privateEnv.SPOOL_DASHBOARD_API_KEY;
    if (!bearerToken) {
      throw new Error(
        'Local live dashboard authentication requires a local-owner session.'
      );
    }
  }

  return { baseUrl, bearerToken };
}

export function dashboardSource(event: Pick<RequestEvent, 'fetch' | 'url' | 'locals'>): DashboardSource {
  const mode = configuredMode();
  if (mode === 'demo') return { mode, api: mockApi };
  const { baseUrl, bearerToken } = dashboardConnection(event);
  return { mode, api: createLiveApi(event.fetch, baseUrl, bearerToken) };
}

export function dashboardSdk(
  event: Pick<RequestEvent, 'fetch' | 'url' | 'locals'>
): SpoolClient {
  const { baseUrl, bearerToken } = dashboardConnection(event);
  return new SpoolClient({
    baseUrl,
    fetch: event.fetch,
    apiKey: bearerToken,
    headers: { 'x-spool-dashboard': '1' }
  });
}

export function dashboardMode(): DashboardMode {
  return configuredMode();
}

export async function dashboardMeta(
  event: Pick<RequestEvent, 'fetch' | 'url' | 'locals'>
): Promise<DashboardMeta> {
  if (configuredMode() === 'demo') return mockApi.meta();
  const baseUrl = publicEnv.PUBLIC_SPOOL_API_URL || event.url.origin;
  try {
    const response = await event.fetch(`${baseUrl.replace(/\/$/, '')}/v1/meta`, {
      headers: { accept: 'application/json' }
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    return parseDashboardMeta(await response.json());
  } catch {
    const hosted = event.locals.authMode === 'workos';
    return {
      deployment: hosted ? 'cloud' : 'self_hosted',
      version: 'unknown',
      auth: {
        provider: hosted ? 'workos' : 'local_owner',
        workspaceSwitching: false,
        invitations: false
      },
      billing: { enabled: false },
      updates: { officialFeed: false, customFeed: !hosted }
    };
  }
}

export function presentDashboardError(error: unknown): DashboardLoadError {
  if (error instanceof SpoolError) {
    return {
      title: 'Control plane request failed',
      message: error.message,
      code: error.code,
      requestId: error.requestId ?? null,
      retryable: error.retryable
    };
  }
  return {
    title: 'Dashboard data unavailable',
    message: error instanceof Error ? error.message : 'An unexpected dashboard error occurred.',
    code: 'dashboard_unavailable',
    requestId: null,
    retryable: true
  };
}
