import { describe, expect, it, vi } from 'vitest';
import { SpoolClient } from '../src/index.js';
import type { Printer } from '../src/index.js';

describe('SpoolClient', () => {
  it('sends auth, idempotency, and JSON for job creation', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          id: 'job_1',
          printer_id: 'printer_1',
          state: 'registered'
        }),
        { status: 201, headers: { 'content-type': 'application/json' } }
      )
    );
    const client = new SpoolClient({
      apiKey: 'spl_test_secret',
      baseUrl: 'https://print.example.test/',
      fetch: fetcher
    });

    await client.jobs.create(
      {
        printer_id: 'printer_1',
        title: 'Packing slip',
        content_type: 'pdf',
        content: { type: 'uri', uri: 'https://example.test/slip.pdf' }
      },
      'order-42'
    );

    const [url, init] = fetcher.mock.calls[0] ?? [];
    expect(String(url)).toBe('https://print.example.test/v1/jobs');
    expect(init?.method).toBe('POST');
    expect(new Headers(init?.headers).get('authorization')).toBe('Bearer spl_test_secret');
    expect(new Headers(init?.headers).get('idempotency-key')).toBe('order-42');
  });

  it('exposes structured API errors', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(
        JSON.stringify({
          error: {
            code: 'printer_unavailable',
            message: 'Printer unavailable',
            request_id: 'req_1',
            retryable: true
          }
        }),
        { status: 409, headers: { 'content-type': 'application/problem+json' } }
      )
    );
    const client = new SpoolClient({ fetch: fetcher });

    await expect(client.jobs.cancel('job_1')).rejects.toMatchObject({
      name: 'SpoolError',
      code: 'printer_unavailable',
      retryable: true
    });
  });

  it('adds cursor filters without serialising undefined values', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ data: [], nextCursor: null }), { status: 200 })
    );
    const client = new SpoolClient({ fetch: fetcher, baseUrl: 'http://localhost:39100' });

    await client.jobs.list({ after: 'job_9', limit: 25 });

    const url = new URL(String(fetcher.mock.calls[0]?.[0]));
    expect(url.origin + url.pathname).toBe('http://localhost:39100/v1/jobs');
    expect(Object.fromEntries(url.searchParams)).toEqual({ after: 'job_9', limit: '25' });
  });

  it('returns complete synced printer capability and profile snapshots', async () => {
    const printer = {
      id: 'printer_1',
      agent_id: 'agent_1',
      name: 'Packing',
      state: 'online',
      capabilities: {
        bins: ['Tray1'],
        collate: true,
        color: true,
        copies: 99,
        dpis: ['300dpi'],
        duplex: true,
        extent: [[2100, 2970]],
        medias: ['plain'],
        nup: [1, 2],
        papers: { A4: [2100, 2970] },
        printrate: { unit: 'ppm', rate: 20 },
        supports_custom_paper_size: true
      },
      capability_revision: 4,
      native_options: {
        InputSlot: {
          display_name: 'Paper source',
          default_choice: 'Tray1',
          selected_choice: 'Tray2',
          choices: [
            { value: 'Tray1', display_name: 'Tray 1' },
            { value: 'Tray2', display_name: 'Tray 2' }
          ]
        }
      },
      profiles: [
        {
          profile_id: 'profile_1',
          revision: 2,
          name: 'Packing slips',
          is_default: true,
          options: { paper: 'A4', native_options: { InputSlot: 'Tray2' } }
        }
      ],
      updated_at: '2026-07-29T00:00:00Z'
    } satisfies Printer;
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ data: [printer], next_cursor: null, has_more: false }))
    );
    const client = new SpoolClient({ fetch: fetcher });

    const page = await client.printers.list();

    expect(page.data[0]?.capability_revision).toBe(4);
    expect(page.data[0]?.native_options.InputSlot?.selected_choice).toBe('Tray2');
    expect(page.data[0]?.profiles[0]).toMatchObject({
      profile_id: 'profile_1',
      name: 'Packing slips'
    });
  });

  it('deletes a webhook through the canonical endpoint', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 204 }));
    const client = new SpoolClient({
      apiKey: 'spl_live_secret',
      fetch: fetcher,
      baseUrl: 'https://api.spool.test'
    });

    await client.webhooks.remove('whk_01K');

    const [url, init] = fetcher.mock.calls[0] ?? [];
    expect(String(url)).toBe('https://api.spool.test/v1/webhooks/whk_01K');
    expect(init?.method).toBe('DELETE');
  });

  it('creates and revokes environment-scoped API keys', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: 'key_1', secret: 'spl_test_x' })))
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: 'key_1', revoked_at: 'now' })));
    const client = new SpoolClient({
      apiKey: 'spl_test_manager',
      fetch: fetcher,
      baseUrl: 'https://print.example.test'
    });

    await client.apiKeys.create({ name: 'CI', scopes: ['jobs_read'] });
    await client.apiKeys.revoke('key_1');

    const [createUrl, createInit] = fetcher.mock.calls[0] ?? [];
    expect(String(createUrl)).toBe('https://print.example.test/v1/api-keys');
    expect(createInit?.method).toBe('POST');
    expect(JSON.parse(String(createInit?.body))).toEqual({
      name: 'CI',
      scopes: ['jobs_read']
    });
    const [revokeUrl, revokeInit] = fetcher.mock.calls[1] ?? [];
    expect(String(revokeUrl)).toBe('https://print.example.test/v1/api-keys/key_1');
    expect(revokeInit?.method).toBe('DELETE');
  });

  it('retrieves the effective billing period and tenant usage without mutation APIs', async () => {
    const usage = {
      period_start: '2026-07-01T00:00:00Z',
      period_end: '2026-08-01T00:00:00Z',
      accepted_live_jobs: 42,
      active_nodes: 2
    };
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        Response.json({
          enabled: true,
          managed_by_platform: false,
          plan: 'pro',
          billing_interval: 'monthly',
          subscription_status: 'active',
          grace_ends_at: null,
          accept_new_cloud_jobs: true,
          entitlement: {
            included_live_jobs: 25_000,
            node_limit: 25,
            metadata_retention_days: 90,
            document_retention_hours: 168,
            overage_job_unit: 1_000,
            overage_price_cents: 25
          },
          usage,
          overage_live_jobs: 0
        })
      )
      .mockResolvedValueOnce(Response.json(usage));
    const client = new SpoolClient({
      apiKey: 'spl_live_usage',
      fetch: fetcher,
      baseUrl: 'https://print.example.test'
    });

    const summary = await client.billing.summary();
    const july = await client.usage.retrieve('2026-07');

    expect(summary.entitlement?.included_live_jobs).toBe(25_000);
    expect(july.accepted_live_jobs).toBe(42);
    expect(String(fetcher.mock.calls[0]?.[0])).toBe(
      'https://print.example.test/v1/billing/summary'
    );
    expect(String(fetcher.mock.calls[1]?.[0])).toBe(
      'https://print.example.test/v1/usage?month=2026-07'
    );
    expect(fetcher.mock.calls.every(([, init]) => init?.method === 'GET')).toBe(true);
  });

  it('uses canonical node routes and keeps pairing codes out of query strings', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: 'node_1', name: 'Packing' })))
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ id: 'dva_1', state: 'approved', expires_at: 'later' }))
      );
    const client = new SpoolClient({
      apiKey: 'spl_live_manager',
      fetch: fetcher,
      baseUrl: 'https://print.example.test'
    });

    await client.nodes.rename('node_1', 'Packing');
    await client.pairing.approve('dva_1', 'ABCD-EFGH');

    expect(String(fetcher.mock.calls[0]?.[0])).toBe(
      'https://print.example.test/v1/nodes/node_1'
    );
    expect(JSON.parse(String(fetcher.mock.calls[1]?.[1]?.body))).toEqual({
      user_code: 'ABCD-EFGH'
    });
    expect(String(fetcher.mock.calls[1]?.[0])).not.toContain('ABCD-EFGH');
  });

  it('keeps local-owner bootstrap tokens in headers and credentials in JSON bodies', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        Response.json({ credential: 'spl_owner_once', workspace: {}, member: {} })
      )
      .mockResolvedValueOnce(
        Response.json({ token: 'spl_session_once', expires_at: '2030-01-01T00:00:00Z' })
      );
    const client = new SpoolClient({
      fetch: fetcher,
      baseUrl: 'https://print.example.test'
    });

    await client.identity.bootstrap(
      { workspace_name: 'Warehouse', email: 'owner@example.com' },
      'bootstrap-secret'
    );
    await client.identity.exchange('spl_owner_once');

    const [bootstrapUrl, bootstrapInit] = fetcher.mock.calls[0] ?? [];
    expect(String(bootstrapUrl)).not.toContain('bootstrap-secret');
    expect(new Headers(bootstrapInit?.headers).get('x-spool-bootstrap-token')).toBe(
      'bootstrap-secret'
    );
    const [exchangeUrl, exchangeInit] = fetcher.mock.calls[1] ?? [];
    expect(String(exchangeUrl)).not.toContain('spl_owner_once');
    expect(JSON.parse(String(exchangeInit?.body))).toEqual({ credential: 'spl_owner_once' });
  });

  it('exposes typed stocks, target bindings, and readiness', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        Response.json([
          {
            id: 'stk_01',
            name: '62 × 29 label',
            sku: 'LABEL-62-29',
            description: null,
            attributes: { kind: 'label', width_mm: 62, height_mm: 29, gap_mm: 3 },
            archived: false,
            created_at: '2026-07-29T00:00:00Z',
            updated_at: '2026-07-29T00:00:00Z'
          }
        ])
      )
      .mockResolvedValueOnce(Response.json([]))
      .mockResolvedValueOnce(
        Response.json({
          target_id: 'tgt_01',
          status: 'target_has_no_ready_binding',
          selected_binding_id: null,
          bindings: []
        })
      );
    const client = new SpoolClient({
      apiKey: 'spl_live_design_app',
      fetch: fetcher,
      baseUrl: 'https://print.example.test'
    });

    const stocks = await client.stocks.list();
    await client.targets.bindings('tgt_01');
    const readiness = await client.targets.readiness('tgt_01');

    expect(stocks[0]?.attributes).toMatchObject({
      kind: 'label',
      width_mm: 62,
      height_mm: 29
    });
    expect(readiness.status).toBe('target_has_no_ready_binding');
    expect(String(fetcher.mock.calls[1]?.[0])).toBe(
      'https://print.example.test/v1/targets/tgt_01/bindings'
    );
  });

  it('uploads declared content through the authenticated proxy without base64', async () => {
    const created = {
      id: 'upl_01',
      object_key: 'hidden/object',
      media_type: 'application/pdf',
      expected_sha256: 'a'.repeat(64),
      expected_bytes: 4,
      state: 'pending',
      expires_at: '2026-07-29T01:00:00Z',
      upload_url: '/v1/uploads/upl_01/content',
      upload_method: 'PUT',
      upload_headers: { 'content-type': 'application/pdf' },
      requires_completion: false
    } as const;
    const completed = { ...created, state: 'complete' } as const;
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(Response.json(created, { status: 201 }))
      .mockResolvedValueOnce(Response.json(completed));
    const client = new SpoolClient({
      apiKey: 'spl_live_upload',
      fetch: fetcher,
      baseUrl: 'https://print.example.test'
    });

    const result = await client.uploads.createAndPut(
      {
        media_type: 'application/pdf',
        byte_length: 4,
        sha256: 'a'.repeat(64)
      },
      new Blob(['%PDF'], { type: 'application/pdf' })
    );

    const [uploadUrl, uploadInit] = fetcher.mock.calls[1] ?? [];
    expect(String(uploadUrl)).toBe('https://print.example.test/v1/uploads/upl_01/content');
    expect(uploadInit?.body).toBeInstanceOf(Blob);
    expect(new Headers(uploadInit?.headers).get('authorization')).toBe(
      'Bearer spl_live_upload'
    );
    expect(result.state).toBe('complete');
  });

  it('never forwards a Spool API key to an absolute signed upload URL', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 200 }));
    const client = new SpoolClient({
      apiKey: 'spl_live_must_not_leak',
      fetch: fetcher,
      baseUrl: 'https://print.example.test'
    });

    await client.uploads.put(
      {
        id: 'upl_01',
        object_key: 'hidden/object',
        media_type: 'application/pdf',
        expected_sha256: 'a'.repeat(64),
        expected_bytes: 4,
        state: 'pending',
        expires_at: '2026-07-29T01:00:00Z',
        upload_url: 'https://storage.example.test/signed',
        upload_method: 'PUT',
        upload_headers: { 'x-upload-token': 'opaque' },
        requires_completion: true
      },
      new Blob(['%PDF'])
    );

    const headers = new Headers(fetcher.mock.calls[0]?.[1]?.headers);
    expect(headers.get('authorization')).toBeNull();
    expect(headers.get('x-upload-token')).toBe('opaque');
  });

  it('omits tenant-selection headers for ordinary API keys', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      Response.json({ data: [], next_cursor: null, has_more: false })
    );
    const client = new SpoolClient({
      apiKey: 'spl_live_tenant',
      fetch: fetcher,
      headers: {
        'X-Spool-Workspace-Id': 'wrk_must_not_escape',
        'x-spool-environment-id': 'env_must_not_escape'
      }
    });

    await client.printers.list();

    const headers = new Headers(fetcher.mock.calls[0]?.[1]?.headers);
    expect(headers.get('x-spool-workspace-id')).toBeNull();
    expect(headers.get('x-spool-environment-id')).toBeNull();
    expect(headers.get('authorization')).toBe('Bearer spl_live_tenant');
  });

  it('rejects tenant credentials combined with a platform context', () => {
    expect(
      () =>
        new SpoolClient({
          apiKey: 'spl_live_tenant',
          platformContext: {
            workspaceId: 'wrk_other',
            environmentId: 'env_other'
          }
        } as never)
    ).toThrow('platformContext requires a distinct platformKey');
  });

  it('adds an explicit grant context only for a platform credential', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      Response.json({ data: [], next_cursor: null, has_more: false })
    );
    const client = new SpoolClient({
      platformKey: 'spl_platform_service_account',
      platformContext: {
        workspaceId: 'wrk_customer_01',
        environmentId: 'env_customer_live'
      },
      fetch: fetcher,
      headers: {
        'x-spool-workspace-id': 'wrk_cannot_override',
        'x-spool-environment-id': 'env_cannot_override'
      }
    });

    await client.jobs.list();

    const headers = new Headers(fetcher.mock.calls[0]?.[1]?.headers);
    expect(headers.get('authorization')).toBe('Bearer spl_platform_service_account');
    expect(headers.get('x-spool-workspace-id')).toBe('wrk_customer_01');
    expect(headers.get('x-spool-environment-id')).toBe('env_customer_live');
  });
});
