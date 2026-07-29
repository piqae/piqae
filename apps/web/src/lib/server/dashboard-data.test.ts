import { beforeEach, describe, expect, it, vi } from 'vitest';

const { publicEnvironment, privateEnvironment } = vi.hoisted(() => ({
  publicEnvironment: {} as Record<string, string>,
  privateEnvironment: {} as Record<string, string>
}));

vi.mock('$env/dynamic/public', () => ({ env: publicEnvironment }));
vi.mock('$env/dynamic/private', () => ({ env: privateEnvironment }));

import { dashboardSource } from './dashboard-data';

const fetcher = vi.fn<typeof fetch>();
const baseEvent = {
  fetch: fetcher,
  url: new URL('https://dashboard.spool.test/dashboard')
};
const oidcAccessToken = 'eyJhbGciOiJSUzI1NiJ9.verified-session-token.signature';

describe('dashboard data source selection', () => {
  beforeEach(() => {
    fetcher.mockReset();
    for (const key of Object.keys(publicEnvironment)) delete publicEnvironment[key];
    for (const key of Object.keys(privateEnvironment)) delete privateEnvironment[key];
    publicEnvironment.PUBLIC_SPOOL_API_URL = 'https://api.spool.test';
  });

  it('forwards a verified hosted session token only in the server-side request', async () => {
    fetcher.mockResolvedValueOnce(
      new Response(JSON.stringify({ data: [], has_more: false }), { status: 200 })
    );
    const source = dashboardSource({
      ...baseEvent,
      locals: {
        authMode: 'workos',
        auth: { accessToken: oidcAccessToken }
      } as never
    });

    await source.api.jobs();

    const [url, init] = fetcher.mock.calls[0] ?? [];
    expect(String(url)).toBe('https://api.spool.test/v1/jobs?limit=100');
    expect(new Headers(init?.headers).get('authorization')).toBe(`Bearer ${oidcAccessToken}`);
    expect(JSON.stringify({ mode: source.mode })).not.toContain(oidcAccessToken);
  });

  it('never falls back to a global service key for hosted users', () => {
    privateEnvironment.SPOOL_DASHBOARD_API_KEY = 'spl_live_must_not_be_used';
    expect(() =>
      dashboardSource({
        ...baseEvent,
        locals: { authMode: 'workos', auth: undefined } as never
      })
    ).toThrow(/does not contain an OIDC access token/);
    expect(fetcher).not.toHaveBeenCalled();
  });

  it('allows an explicit server-only service key in local/self-host mode', async () => {
    privateEnvironment.SPOOL_DASHBOARD_API_KEY = 'spl_live_local_service_key';
    fetcher.mockResolvedValueOnce(
      new Response(JSON.stringify({ data: [], has_more: false }), { status: 200 })
    );
    const source = dashboardSource({
      ...baseEvent,
      locals: { authMode: 'local' } as never
    });

    await source.api.jobs();

    const [, init] = fetcher.mock.calls[0] ?? [];
    expect(new Headers(init?.headers).get('authorization')).toBe(
      'Bearer spl_live_local_service_key'
    );
    expect(JSON.stringify({ mode: source.mode })).not.toContain('spl_live_local_service_key');
  });

  it('uses deterministic demo data only after explicit opt-in', async () => {
    publicEnvironment.PUBLIC_SPOOL_DASHBOARD_MODE = 'demo';
    const source = dashboardSource({
      ...baseEvent,
      locals: { authMode: 'demo' } as never
    });
    expect(source.mode).toBe('demo');
    await expect(source.api.jobs()).resolves.toMatchObject({ data: expect.any(Array) });
    expect(fetcher).not.toHaveBeenCalled();
  });

  it('does not log bearer credentials while creating or using a source', async () => {
    const consoleSpies = [
      vi.spyOn(console, 'log').mockImplementation(() => undefined),
      vi.spyOn(console, 'info').mockImplementation(() => undefined),
      vi.spyOn(console, 'warn').mockImplementation(() => undefined),
      vi.spyOn(console, 'error').mockImplementation(() => undefined)
    ];
    fetcher.mockResolvedValueOnce(
      new Response(JSON.stringify({ data: [], has_more: false }), { status: 200 })
    );

    const source = dashboardSource({
      ...baseEvent,
      locals: {
        authMode: 'workos',
        auth: { accessToken: oidcAccessToken }
      } as never
    });
    await source.api.jobs();

    for (const spy of consoleSpies) {
      expect(spy).not.toHaveBeenCalled();
      spy.mockRestore();
    }
  });
});
