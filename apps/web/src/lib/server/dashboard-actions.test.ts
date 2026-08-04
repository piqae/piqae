import { beforeEach, describe, expect, it, vi } from 'vitest';

const { publicEnvironment, privateEnvironment } = vi.hoisted(() => ({
  publicEnvironment: {} as Record<string, string>,
  privateEnvironment: {} as Record<string, string>
}));

vi.mock('$env/dynamic/public', () => ({ env: publicEnvironment }));
vi.mock('$env/dynamic/private', () => ({ env: privateEnvironment }));

// Node enrolment and job cancellation live on the operations page; credential
// and webhook mutations live on the merged settings page.
import { actions as operationsActions } from '../../routes/dashboard/+page.server';
import { actions as settingsActions } from '../../routes/dashboard/settings/+page.server';

const agentActions = operationsActions;
const jobActions = operationsActions;
const apiKeyActions = settingsActions;
const webhookActions = settingsActions;

const accessToken = 'eyJ.test-hosted-access-token.signature';
const fetcher = vi.fn<typeof fetch>();
const setHeaders = vi.fn();

function actionEvent(fields: Record<string, string | string[]> = {}, params = {}) {
  const body = new URLSearchParams();
  for (const [name, value] of Object.entries(fields)) {
    for (const item of Array.isArray(value) ? value : [value]) body.append(name, item);
  }
  return {
    fetch: fetcher,
    setHeaders,
    request: new Request('https://dashboard.piqae.test/dashboard', {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: body.toString()
    }),
    url: new URL('https://dashboard.piqae.test/dashboard'),
    locals: {
      authMode: 'workos',
      auth: { accessToken }
    },
    params
  };
}

function requestDetails(call = 0) {
  const [url, init] = fetcher.mock.calls[call] ?? [];
  return {
    url: String(url),
    method: init?.method,
    authorization: new Headers(init?.headers).get('authorization'),
    body: init?.body ? JSON.parse(String(init.body)) : undefined
  };
}

describe('dashboard mutation actions', () => {
  beforeEach(() => {
    fetcher.mockReset();
    setHeaders.mockReset();
    for (const key of Object.keys(publicEnvironment)) delete publicEnvironment[key];
    for (const key of Object.keys(privateEnvironment)) delete privateEnvironment[key];
    publicEnvironment.PUBLIC_PIQAE_API_URL = 'https://api.piqae.test';
    publicEnvironment.PUBLIC_PIQAE_DASHBOARD_MODE = 'live';
  });

  it('creates a native connection invitation with hosted auth kept out of the result', async () => {
    fetcher.mockResolvedValueOnce(
      Response.json({
        id: 'enr_01',
        state: 'pending',
        expires_at: '2026-07-29T12:10:00.000Z',
        node_id: null,
        connect_url: `https://app.piqae.com/connect#enrolment_token=piq_enr_${'a'.repeat(32)}`,
        downloads: []
      })
    );

    const result = await agentActions.createEnrolment!(
      actionEvent({ name: 'Warehouse agent', expires_in_seconds: '600' }) as never
    );

    expect(requestDetails()).toEqual({
      url: 'https://api.piqae.test/v1/node-connect-sessions',
      method: 'POST',
      authorization: `Bearer ${accessToken}`,
      body: {
        name: 'Warehouse agent',
        expires_in_seconds: 600,
        return_url: 'https://dashboard.piqae.test/dashboard?view=nodes'
      }
    });
    expect(result).toMatchObject({
      mutation: 'createEnrolment',
      enrolment: { connectUrl: expect.stringContaining('https://app.piqae.com/connect#') }
    });
    expect(setHeaders).toHaveBeenCalledWith({ 'cache-control': 'no-store, private' });
    expect(JSON.stringify(result)).not.toContain(accessToken);
  });

  it('creates and deletes webhooks without exposing dashboard credentials', async () => {
    fetcher
      .mockResolvedValueOnce(
        Response.json({
          id: 'whk_01',
          url: 'https://example.test/piqae',
          events: ['job.*'],
          secret: 'whsec_once'
        })
      )
      .mockResolvedValueOnce(new Response(null, { status: 204 }));

    const created = await webhookActions.createWebhook!(
      actionEvent({ url: 'https://example.test/piqae', events: ['job.*'] }) as never
    );
    const deleted = await webhookActions.deleteWebhook!(
      actionEvent({ webhook_id: 'whk_01' }) as never
    );

    expect(requestDetails(0)).toMatchObject({
      url: 'https://api.piqae.test/v1/webhooks',
      method: 'POST',
      authorization: `Bearer ${accessToken}`,
      body: { url: 'https://example.test/piqae', events: ['job.*'] }
    });
    expect(requestDetails(1)).toMatchObject({
      url: 'https://api.piqae.test/v1/webhooks/whk_01',
      method: 'DELETE',
      authorization: `Bearer ${accessToken}`
    });
    expect(created).toMatchObject({ webhook: { secret: 'whsec_once' } });
    expect(setHeaders).toHaveBeenCalledWith({ 'cache-control': 'no-store, private' });
    expect(deleted).toEqual({ mutation: 'deleteWebhook', deletedWebhookId: 'whk_01' });
    expect(JSON.stringify([created, deleted])).not.toContain(accessToken);
  });

  it('forwards only allowlisted API-key scopes and supports explicit revocation', async () => {
    fetcher
      .mockResolvedValueOnce(
        Response.json({
          id: 'key_01',
          name: 'Order service',
          lookup_prefix: 'piq_live_abcd',
          scopes: ['jobs_read', 'jobs_write'],
          expires_at: null,
          last_used_at: null,
          revoked_at: null,
          created_at: '2026-07-29T12:00:00.000Z',
          secret: 'piq_live_once'
        })
      )
      .mockResolvedValueOnce(
        Response.json({
          id: 'key_01',
          name: 'Order service',
          lookup_prefix: 'piq_live_abcd',
          scopes: ['jobs_read', 'jobs_write'],
          expires_at: null,
          last_used_at: null,
          revoked_at: '2026-07-29T12:01:00.000Z',
          created_at: '2026-07-29T12:00:00.000Z'
        })
      );

    const created = await apiKeyActions.createApiKey!(
      actionEvent({
        name: 'Order service',
        scopes: ['jobs_read', 'jobs_write', 'jobs_read']
      }) as never
    );
    const revoked = await apiKeyActions.revokeApiKey!(
      actionEvent({ api_key_id: 'key_01' }) as never
    );

    expect(requestDetails(0)).toMatchObject({
      url: 'https://api.piqae.test/v1/api-keys',
      method: 'POST',
      authorization: `Bearer ${accessToken}`,
      body: {
        name: 'Order service',
        scopes: ['jobs_read', 'jobs_write'],
        expires_at: null
      }
    });
    expect(requestDetails(1)).toMatchObject({
      url: 'https://api.piqae.test/v1/api-keys/key_01',
      method: 'DELETE'
    });
    expect(created).toMatchObject({ apiKey: { secret: 'piq_live_once' } });
    expect(setHeaders).toHaveBeenCalledWith({ 'cache-control': 'no-store, private' });
    expect(revoked).toEqual({ mutation: 'revokeApiKey', revokedApiKeyId: 'key_01' });
    expect(JSON.stringify([created, revoked])).not.toContain(accessToken);
  });

  it('rejects unknown API scopes before contacting the control plane', async () => {
    const result = await apiKeyActions.createApiKey!(
      actionEvent({ name: 'Unsafe key', scopes: ['admin_everything'] }) as never
    );

    expect(result).toMatchObject({ status: 400 });
    expect(fetcher).not.toHaveBeenCalled();
  });

  it('cancels a job through the server-side SDK and returns only state', async () => {
    fetcher.mockResolvedValueOnce(
      Response.json({ id: 'job_01', state: 'cancelled', title: 'Packing slip' })
    );

    const result = await jobActions.cancelJob!(actionEvent({ job_id: 'job_01' }) as never);

    expect(requestDetails()).toMatchObject({
      url: 'https://api.piqae.test/v1/jobs/job_01/cancel',
      method: 'POST',
      authorization: `Bearer ${accessToken}`
    });
    expect(result).toEqual({
      mutation: 'cancelJob',
      cancelledJobId: 'job_01',
      state: 'cancelled'
    });
    expect(JSON.stringify(result)).not.toContain(accessToken);
  });

  it('keeps demo actions deterministic and non-mutating', async () => {
    publicEnvironment.PUBLIC_PIQAE_DASHBOARD_MODE = 'demo';

    const results = await Promise.all([
      agentActions.createEnrolment!(actionEvent({ name: 'Demo agent' }) as never),
      webhookActions.createWebhook!(
        actionEvent({ url: 'https://example.test', events: ['job.*'] }) as never
      ),
      apiKeyActions.createApiKey!(
        actionEvent({ name: 'Demo key', scopes: ['jobs_read'] }) as never
      ),
      jobActions.cancelJob!(actionEvent({ job_id: 'job_demo' }) as never)
    ]);

    for (const result of results) expect(result).toMatchObject({ status: 400 });
    expect(fetcher).not.toHaveBeenCalled();
  });

  it('does not log access tokens or newly issued secrets', async () => {
    const consoleSpies = [
      vi.spyOn(console, 'log').mockImplementation(() => undefined),
      vi.spyOn(console, 'info').mockImplementation(() => undefined),
      vi.spyOn(console, 'warn').mockImplementation(() => undefined),
      vi.spyOn(console, 'error').mockImplementation(() => undefined)
    ];
    fetcher.mockResolvedValueOnce(
      Response.json({
        id: 'enr_02',
        token: 'piqae_enrol_private',
        expires_at: '2026-07-29T12:10:00.000Z'
      })
    );

    await agentActions.createEnrolment!(actionEvent({ name: 'Private agent' }) as never);

    for (const spy of consoleSpies) {
      expect(spy).not.toHaveBeenCalled();
      spy.mockRestore();
    }
  });
});
