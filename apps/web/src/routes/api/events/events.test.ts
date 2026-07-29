import { beforeEach, describe, expect, it, vi } from 'vitest';

const { publicEnvironment, privateEnvironment } = vi.hoisted(() => ({
  publicEnvironment: {} as Record<string, string>,
  privateEnvironment: {} as Record<string, string>
}));

vi.mock('$env/dynamic/public', () => ({ env: publicEnvironment }));
vi.mock('$env/dynamic/private', () => ({ env: privateEnvironment }));

import { GET } from './+server';

const accessToken = 'eyJhbGciOiJSUzI1NiJ9.server-only-oidc-token.signature';

describe('same-origin event proxy', () => {
  beforeEach(() => {
    for (const key of Object.keys(publicEnvironment)) delete publicEnvironment[key];
    for (const key of Object.keys(privateEnvironment)) delete privateEnvironment[key];
    publicEnvironment.PUBLIC_SPOOL_DASHBOARD_MODE = 'live';
    publicEnvironment.PUBLIC_SPOOL_API_URL = 'https://api.spool.test';
  });

  it('forwards session auth and SSE cursors server-side without exposing the token', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response('id: evt_2\nevent: job.updated\ndata: {"id":"job_1"}\n\n', {
        status: 200,
        headers: { 'content-type': 'text/event-stream' }
      })
    );
    const response = await GET({
      fetch: fetcher,
      url: new URL('https://dashboard.spool.test/api/events'),
      request: new Request('https://dashboard.spool.test/api/events', {
        headers: { 'last-event-id': 'evt_1' }
      }),
      locals: {
        authMode: 'workos',
        auth: { accessToken }
      }
    } as never);

    const [url, init] = fetcher.mock.calls[0] ?? [];
    const upstreamHeaders = new Headers(init?.headers);
    expect(String(url)).toBe('https://api.spool.test/v1/events/stream');
    expect(upstreamHeaders.get('authorization')).toBe(`Bearer ${accessToken}`);
    expect(upstreamHeaders.get('last-event-id')).toBe('evt_1');
    expect(response.headers.get('content-type')).toBe('text/event-stream');
    expect(response.headers.get('cache-control')).toBe('no-cache, no-transform');
    const body = await response.text();
    expect(body).toContain('event: job.updated');
    expect(body).not.toContain(accessToken);
  });
});
