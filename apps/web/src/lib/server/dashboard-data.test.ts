import { beforeEach, describe, expect, it, vi } from 'vitest';

const { publicEnvironment, privateEnvironment } = vi.hoisted(() => ({
  publicEnvironment: {} as Record<string, string>,
  privateEnvironment: {} as Record<string, string>
}));

vi.mock('$env/dynamic/public', () => ({ env: publicEnvironment }));
vi.mock('$env/dynamic/private', () => ({ env: privateEnvironment }));

import { dashboardSource } from './dashboard-data';

const event = {
  fetch: vi.fn<typeof fetch>(),
  url: new URL('https://dashboard.spool.test/dashboard')
};

describe('dashboard data source selection', () => {
  beforeEach(() => {
    event.fetch.mockReset();
    for (const key of Object.keys(publicEnvironment)) delete publicEnvironment[key];
    for (const key of Object.keys(privateEnvironment)) delete privateEnvironment[key];
  });

  it('uses live mode by default and requires a server-only credential', () => {
    expect(() => dashboardSource(event)).toThrow(/SPOOL_DASHBOARD_API_KEY/);
  });

  it('uses deterministic demo data only after explicit opt-in', async () => {
    publicEnvironment.PUBLIC_SPOOL_DASHBOARD_MODE = 'demo';
    const source = dashboardSource(event);
    expect(source.mode).toBe('demo');
    await expect(source.api.jobs()).resolves.toMatchObject({ data: expect.any(Array) });
    expect(event.fetch).not.toHaveBeenCalled();
  });

  it('does not place a live API key into page data or request URLs', async () => {
    publicEnvironment.PUBLIC_SPOOL_DASHBOARD_MODE = 'live';
    publicEnvironment.PUBLIC_SPOOL_API_URL = 'https://api.spool.test';
    privateEnvironment.SPOOL_DASHBOARD_API_KEY = 'spl_live_server_only';
    event.fetch.mockResolvedValueOnce(
      new Response(JSON.stringify({ data: [], has_more: false }), { status: 200 })
    );
    const source = dashboardSource(event);
    await source.api.jobs();
    const [url, init] = event.fetch.mock.calls[0] ?? [];
    expect(String(url)).toBe('https://api.spool.test/v1/jobs?limit=100');
    expect(new Headers(init?.headers).get('authorization')).toBe('Bearer spl_live_server_only');
    expect(JSON.stringify({ mode: source.mode })).not.toContain('spl_live_server_only');
  });
});
