import { env as privateEnv } from '$env/dynamic/private';
import { env as publicEnv } from '$env/dynamic/public';
import type { RequestEvent } from '@sveltejs/kit';
import { SpoolError } from '@spool/sdk';
import { createLiveApi, mockApi, type DashboardApi } from '$lib/api';

export type DashboardMode = 'live' | 'demo';

export interface DashboardSource {
  api: DashboardApi;
  mode: DashboardMode;
}

export interface DashboardLoadError {
  title: string;
  message: string;
  code: string;
  requestId: string | null;
  retryable: boolean;
}

function configuredMode(): DashboardMode {
  const value = publicEnv.PUBLIC_SPOOL_DASHBOARD_MODE;
  if (value === 'demo') return 'demo';
  if (value === undefined || value === '' || value === 'live') return 'live';
  throw new Error('PUBLIC_SPOOL_DASHBOARD_MODE must be live or demo');
}

export function dashboardSource(event: Pick<RequestEvent, 'fetch' | 'url'>): DashboardSource {
  const mode = configuredMode();
  if (mode === 'demo') return { mode, api: mockApi };

  const baseUrl = publicEnv.PUBLIC_SPOOL_API_URL || event.url.origin;
  const apiKey = privateEnv.SPOOL_DASHBOARD_API_KEY;
  if (!apiKey) {
    throw new Error(
      'Live dashboard authentication is unavailable. Configure the server-only ' +
        'SPOOL_DASHBOARD_API_KEY until control-plane session exchange is implemented.'
    );
  }
  return { mode, api: createLiveApi(event.fetch, baseUrl, apiKey) };
}

export function dashboardMode(): DashboardMode {
  return configuredMode();
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
