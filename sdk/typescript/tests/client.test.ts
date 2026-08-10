import { describe, expect, it, vi } from 'vitest';
import { PiqaeClient, verifyWebhookSignature } from '../src/index.js';
import type { EncryptedJobEnvelope, Printer } from '../src/index.js';

describe('PiqaeClient', () => {
  it('defaults hosted clients to the canonical Piqae API origin', () => {
    expect(new PiqaeClient().baseUrl).toBe('https://api.piqae.com');
  });

  it('keeps optional document rendering separate and sends idempotency', async () => {
    const fetcher = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(Response.json({ id: 'rnd_1', template_revision_id: 'rev_1', state: 'completed', created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z' }, { status: 202 }))
      .mockResolvedValueOnce(Response.json({ id: 'job_1', state: 'registered' }, { status: 201 }));
    const client = new PiqaeClient({ fetch: fetcher });
    const render = await client.documents.renders.create({ template_revision_id: 'rev_1', input: { invoice_number: 'redacted' } }, 'render-0001');
    await client.documents.renders.print(render.id, { target_id: 'tgt_1', title: 'Invoice' }, 'print-0001');
    expect(String(fetcher.mock.calls[0]?.[0])).toBe('https://api.piqae.com/v1/document-renders');
    expect(new Headers(fetcher.mock.calls[0]?.[1]?.headers).get('idempotency-key')).toBe('render-0001');
    expect(String(fetcher.mock.calls[1]?.[0])).toBe('https://api.piqae.com/v1/document-renders/rnd_1/print');
    expect(new Headers(fetcher.mock.calls[1]?.[1]?.headers).get('idempotency-key')).toBe('print-0001');
  });

  it('downloads verified render PDFs as a Response or bytes for same-origin proxies', async () => {
    const pdf = new TextEncoder().encode('%PDF-fixture');
    const fetcher = vi.fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(pdf, { headers: { 'content-type': 'application/pdf' } }))
      .mockResolvedValueOnce(new Response(pdf, { headers: { 'content-type': 'application/pdf' } }));
    const client = new PiqaeClient({ apiKey: 'piq_test_redacted', fetch: fetcher });
    const response = await client.documents.renders.download('drnd_1');
    expect(response.headers.get('content-type')).toBe('application/pdf');
    expect(await client.documents.renders.downloadBytes('drnd_1')).toEqual(pdf);
    expect(String(fetcher.mock.calls[0]?.[0])).toBe(
      'https://api.piqae.com/v1/document-renders/drnd_1/artifact'
    );
    expect(new Headers(fetcher.mock.calls[0]?.[1]?.headers).get('accept')).toBe('application/pdf');
    expect(new Headers(fetcher.mock.calls[0]?.[1]?.headers).get('authorization')).toBe(
      'Bearer piq_test_redacted'
    );
  });

  it('maps the conversion SDK camelCase version to the wire contract', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(Response.json({ id: 'dcnv_1' }, { status: 201 }));
    const client = new PiqaeClient({ fetch: fetcher });
    await client.documents.conversions.create({
      adapter: 'pdfme', adapterVersion: '1.0.0', strict: true,
      source: { basePdf: { width: 210, height: 297 }, schemas: [] }
    }, 'conversion-template-v1');
    const [, init] = fetcher.mock.calls[0] ?? [];
    expect(JSON.parse(String(init?.body))).toMatchObject({ adapter_version: '1.0.0' });
    expect(new Headers(init?.headers).get('idempotency-key')).toBe('conversion-template-v1');
  });

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
    const client = new PiqaeClient({
      apiKey: 'piq_test_secret',
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
    expect(new Headers(init?.headers).get('authorization')).toBe('Bearer piq_test_secret');
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
    const client = new PiqaeClient({ fetch: fetcher });

    await expect(client.jobs.cancel('job_1')).rejects.toMatchObject({
      name: 'PiqaeError',
      code: 'printer_unavailable',
      retryable: true
    });
  });

  it('adds cursor filters without serialising undefined values', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ data: [], nextCursor: null }), { status: 200 })
    );
    const client = new PiqaeClient({ fetch: fetcher, baseUrl: 'http://localhost:39100' });

    await client.jobs.list({ after: 'job_9', limit: 25 });

    const url = new URL(String(fetcher.mock.calls[0]?.[0]));
    expect(url.origin + url.pathname).toBe('http://localhost:39100/v1/jobs');
    expect(Object.fromEntries(url.searchParams)).toEqual({ after: 'job_9', limit: '25' });
  });

  it('adds exact job reconciliation filters', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ data: [], next_cursor: null, has_more: false }), { status: 200 })
    );
    const client = new PiqaeClient({ fetch: fetcher });
    await client.jobs.list({
      state: 'failed_retryable',
      printer_id: 'printer_1',
      target_id: 'target_1',
      metadata_key: 'order_id',
      metadata_value: 'order_42'
    });
    const url = new URL(String(fetcher.mock.calls[0]?.[0]));
    expect(Object.fromEntries(url.searchParams)).toMatchObject({
      state: 'failed_retryable', printer_id: 'printer_1', target_id: 'target_1',
      metadata_key: 'order_id', metadata_value: 'order_42'
    });
  });

  it('retrieves a consolidated design specification', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ specification_revision: 'spec_1' }), { status: 200 })
    );
    const client = new PiqaeClient({ fetch: fetcher });
    await client.targets.designSpecification('target / one');
    expect(String(fetcher.mock.calls[0]?.[0])).toBe(
      'https://api.piqae.com/v1/targets/target%20%2F%20one/design-specification'
    );
  });

  it('confirms or clears a printer loaded-media observation', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      new Response(JSON.stringify({ source: 'main-roll', stock: null }), { status: 200 })
    );
    const client = new PiqaeClient({ fetch: fetcher });

    await client.printers.confirmLoadedMedia('printer / one', {
      source: 'main-roll',
      stock: null,
      calibration_state: 'unknown'
    });

    const [url, init] = fetcher.mock.calls[0] ?? [];
    expect(String(url)).toBe(
      'https://api.piqae.com/v1/printers/printer%20%2F%20one/loaded-media'
    );
    expect(init?.method).toBe('PUT');
    expect(JSON.parse(String(init?.body))).toEqual({
      source: 'main-roll',
      stock: null,
      calibration_state: 'unknown'
    });
  });

  it('verifies signed webhooks and rejects stale or changed bodies', async () => {
    const timestamp = 1_700_000_000;
    const body = '{"id":"evt_1"}';
    const key = await crypto.subtle.importKey(
      'raw', new TextEncoder().encode('whsec_test'), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']
    );
    const digest = new Uint8Array(await crypto.subtle.sign(
      'HMAC', key, new TextEncoder().encode(`${timestamp}.${body}`)
    ));
    const signature = btoa(String.fromCharCode(...digest));
    const headers = { 'piqae-timestamp': String(timestamp), 'piqae-signature': `v1=${signature}` };
    await expect(verifyWebhookSignature('whsec_test', body, headers, { now: timestamp * 1000 }))
      .resolves.toBe(true);
    await expect(verifyWebhookSignature('whsec_test', `${body} `, headers, { now: timestamp * 1000 }))
      .resolves.toBe(false);
    await expect(verifyWebhookSignature('whsec_test', body, headers, { now: (timestamp + 301) * 1000 }))
      .resolves.toBe(false);
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
    const client = new PiqaeClient({ fetch: fetcher });

    const page = await client.printers.list();

    expect(page.data[0]?.capability_revision).toBe(4);
    expect(page.data[0]?.native_options.InputSlot?.selected_choice).toBe('Tray2');
    expect(page.data[0]?.profiles[0]).toMatchObject({
      profile_id: 'profile_1',
      name: 'Packing slips'
    });
  });

  it('manages webhook endpoints, delivery history, and replay', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(Response.json([]))
      .mockResolvedValueOnce(new Response(null, { status: 202 }))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    const client = new PiqaeClient({
      apiKey: 'piq_live_secret',
      fetch: fetcher,
      baseUrl: 'https://api.piqae.test'
    });

    await client.webhooks.deliveries('whk_01K');
    await client.webhooks.replay('whd_01K');
    await client.webhooks.remove('whk_01K');

    const [deliveryUrl, deliveryInit] = fetcher.mock.calls[0] ?? [];
    expect(String(deliveryUrl)).toBe(
      'https://api.piqae.test/v1/webhooks/whk_01K/deliveries'
    );
    expect(deliveryInit?.method).toBe('GET');
    const [replayUrl, replayInit] = fetcher.mock.calls[1] ?? [];
    expect(String(replayUrl)).toBe(
      'https://api.piqae.test/v1/webhook-deliveries/whd_01K/replay'
    );
    expect(replayInit?.method).toBe('POST');
    const [url, init] = fetcher.mock.calls[2] ?? [];
    expect(String(url)).toBe('https://api.piqae.test/v1/webhooks/whk_01K');
    expect(init?.method).toBe('DELETE');
  });

  it('creates and revokes environment-scoped API keys', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: 'key_1', secret: 'piq_test_x' })))
      .mockResolvedValueOnce(new Response(JSON.stringify({ id: 'key_1', revoked_at: 'now' })));
    const client = new PiqaeClient({
      apiKey: 'piq_test_manager',
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
      reported_complete_live_jobs: 42,
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
    const client = new PiqaeClient({
      apiKey: 'piq_live_usage',
      fetch: fetcher,
      baseUrl: 'https://print.example.test'
    });

    const summary = await client.billing.summary();
    const july = await client.usage.retrieve('2026-07');

    expect(summary.entitlement?.included_live_jobs).toBe(25_000);
    expect(july.reported_complete_live_jobs).toBe(42);
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
    const client = new PiqaeClient({
      apiKey: 'piq_live_manager',
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

  it('lists and revokes only explicitly addressed node connectors', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(Response.json([]))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    const client = new PiqaeClient({
      apiKey: 'piq_live_manager',
      fetch: fetcher,
      baseUrl: 'https://print.example.test'
    });

    await client.nodes.connectors('node/one');
    await client.nodes.revokeConnector('node/one', 'ncon/one');

    expect(String(fetcher.mock.calls[0]?.[0])).toBe(
      'https://print.example.test/v1/nodes/node%2Fone/connectors'
    );
    expect(String(fetcher.mock.calls[1]?.[0])).toBe(
      'https://print.example.test/v1/nodes/node%2Fone/connectors/ncon%2Fone'
    );
    expect(fetcher.mock.calls[1]?.[1]?.method).toBe('DELETE');
  });

  it('keeps local-owner bootstrap tokens in headers and credentials in JSON bodies', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(
        Response.json({ credential: 'piq_owner_once', workspace: {}, member: {} })
      )
      .mockResolvedValueOnce(
        Response.json({ token: 'piq_session_once', expires_at: '2030-01-01T00:00:00Z' })
      );
    const client = new PiqaeClient({
      fetch: fetcher,
      baseUrl: 'https://print.example.test'
    });

    await client.identity.bootstrap(
      { workspace_name: 'Warehouse', email: 'owner@example.com' },
      'bootstrap-secret'
    );
    await client.identity.exchange('piq_owner_once');

    const [bootstrapUrl, bootstrapInit] = fetcher.mock.calls[0] ?? [];
    expect(String(bootstrapUrl)).not.toContain('bootstrap-secret');
    expect(new Headers(bootstrapInit?.headers).get('x-piqae-bootstrap-token')).toBe(
      'bootstrap-secret'
    );
    const [exchangeUrl, exchangeInit] = fetcher.mock.calls[1] ?? [];
    expect(String(exchangeUrl)).not.toContain('piq_owner_once');
    expect(JSON.parse(String(exchangeInit?.body))).toEqual({ credential: 'piq_owner_once' });
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
    const client = new PiqaeClient({
      apiKey: 'piq_live_design_app',
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
    const client = new PiqaeClient({
      apiKey: 'piq_live_upload',
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
      'Bearer piq_live_upload'
    );
    expect(result.state).toBe('complete');
  });

  it('never forwards a Piqae API key to an absolute signed upload URL', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 200 }));
    const client = new PiqaeClient({
      apiKey: 'piq_live_must_not_leak',
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
    const client = new PiqaeClient({
      apiKey: 'piq_live_tenant',
      fetch: fetcher,
      headers: {
        'X-Piqae-Workspace-Id': 'wrk_must_not_escape',
        'x-piqae-environment-id': 'env_must_not_escape'
      }
    });

    await client.printers.list();

    const headers = new Headers(fetcher.mock.calls[0]?.[1]?.headers);
    expect(headers.get('x-piqae-workspace-id')).toBeNull();
    expect(headers.get('x-piqae-environment-id')).toBeNull();
    expect(headers.get('authorization')).toBe('Bearer piq_live_tenant');
  });

  it('rejects tenant credentials combined with a platform context', () => {
    expect(
      () =>
        new PiqaeClient({
          apiKey: 'piq_live_tenant',
          platformContext: {
            workspaceId: 'wrk_other',
            environmentId: 'env_other'
          }
        } as never)
    ).toThrow('platformContext requires a distinct platformKey');
  });

  it('fails clearly when server-side code opens SSE without an EventSource implementation', () => {
    const client = new PiqaeClient();
    expect(() => client.events()).toThrow(
      'PiqaeClient.events requires a browser EventSource or the eventSource constructor option'
    );
  });

  it('adds an explicit grant context only for a platform credential', async () => {
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      Response.json({ data: [], next_cursor: null, has_more: false })
    );
    const client = new PiqaeClient({
      platformKey: 'piq_platform_service_account',
      platformContext: {
        workspaceId: 'wrk_customer_01',
        environmentId: 'env_customer_live'
      },
      fetch: fetcher,
      headers: {
        'x-piqae-workspace-id': 'wrk_cannot_override',
        'x-piqae-environment-id': 'env_cannot_override'
      }
    });

    await client.jobs.list();

    const headers = new Headers(fetcher.mock.calls[0]?.[1]?.headers);
    expect(headers.get('authorization')).toBe('Bearer piq_platform_service_account');
    expect(headers.get('x-piqae-workspace-id')).toBe('wrk_customer_01');
    expect(headers.get('x-piqae-environment-id')).toBe('env_customer_live');
  });

  // A device code in a URL is recorded by every proxy, CDN, and gateway access
  // log between the caller and the control plane, none of which Piqae controls.
  it('never places a pairing device code in a request URL', async () => {
    const deviceCode = 'piq_dev_secret_pairing_capability';
    const fetcher = vi
      .fn<typeof fetch>()
      .mockImplementation(async () =>
        Response.json({ id: 'dva_1', state: 'pending', expires_at: null })
      );
    const client = new PiqaeClient({ baseUrl: 'https://print.example.test/', fetch: fetcher });

    await client.pairing.status(deviceCode);
    await client.pairing.exchange(deviceCode);

    for (const [url, init] of fetcher.mock.calls) {
      expect(String(url)).not.toContain(deviceCode);
      expect(init?.method).toBe('POST');
      expect(JSON.parse(String(init?.body))).toEqual({ device_code: deviceCode });
    }
    expect(String(fetcher.mock.calls[0]?.[0])).toBe(
      'https://print.example.test/v1/device-authorizations/status'
    );
    expect(String(fetcher.mock.calls[1]?.[0])).toBe(
      'https://print.example.test/v1/device-authorizations/exchange'
    );
  });

  it('requires and forwards idempotency for encrypted job registration', async () => {
    const upload = { id: 'upl_cipher', media_type: 'application/octet-stream', expected_sha256: '00'.repeat(32), expected_bytes: 17, state: 'complete', expires_at: '2099-01-01T00:00:00Z' };
    const fetcher = vi.fn<typeof fetch>().mockImplementation(async (url, init) => {
      if (String(url).endsWith('/v1/uploads') && init?.method === 'POST') return Response.json({ ...upload, upload_url: '/v1/uploads/upl_cipher/content', upload_method: 'PUT', upload_headers: {}, requires_completion: false }, { status: 201 });
      if (String(url).includes('/content')) return Response.json(upload);
      return Response.json({ id: 'job_cipher', printer_id: 'prt_1', state: 'registered' }, { status: 201 });
    });
    const client = new PiqaeClient({ baseUrl: 'https://print.example.test/', fetch: fetcher });
    const envelope: EncryptedJobEnvelope = { version: 'piqae-encrypted-job-v3', suite: 'ECDH-ES-P256+HKDF-SHA256+A256GCMKW+A256GCM', binding: { envelope_id: 'env_012345678901234567890123', workspace_id: 'wsp_1', environment_id: 'env_1', content_type: 'pdf', printer_id: 'prt_1', target_id: 'tgt_1', profile_revision: 'prf_1:1', options: { bin: null, collate: null, color: null, copies: null, dpi: null, duplex: null, fit_to_page: null, media: null, nup: null, pages: null, paper: null, rotate: null, native_options: {} }, deliveries: 1, expires_at: '2099-01-01T00:00:00Z', raw_authorized: false }, ciphertext_sha256: 'A'.repeat(43), iv: 'A'.repeat(16), ciphertext: 'AQ', recipients: [{ key_id: 'cek_1', algorithm: 'ECDH-ES-P256+HKDF-SHA256+A256GCMKW', ephemeral_public_key: `B${'A'.repeat(86)}`, hkdf_salt: 'A'.repeat(43), key_wrap_iv: 'A'.repeat(16), encrypted_content_key: 'A'.repeat(64) }] };
    await client.jobs.createEncrypted({ target_id: 'tgt_1', title: 'Private', content_type: 'pdf' }, envelope, 'encrypted-retry-1');
    const jobCall = fetcher.mock.calls.find(([url]) => String(url).endsWith('/v1/jobs'));
    expect(new Headers(jobCall?.[1]?.headers).get('idempotency-key')).toBe('encrypted-retry-1');
    await expect(client.jobs.createEncrypted({ target_id: 'tgt_1', title: 'Private', content_type: 'pdf' }, envelope, 'short')).rejects.toThrow(/Idempotency-Key/);
    await expect(client.jobs.createEncrypted({ target_id: 'tgt_1', title: 'Private', content_type: 'pdf' }, envelope, '🔐'.repeat(64))).rejects.toThrow(/Idempotency-Key/);
    await expect(client.jobs.createEncrypted({ target_id: 'tgt_1', title: 'Unsafe', content_type: 'pdf', resolved_ticket_digest: 'b'.repeat(64) } as never, envelope, 'encrypted-unsafe-1')).rejects.toThrow(/cannot attach unbound/);
    await expect(client.jobs.createEncryptedResolved(
      { target_id: 'tgt_1', title: 'Private', content_type: 'pdf' },
      envelope,
      { digest: 'c'.repeat(64), printer_id: 'prt_other', capability_revision: 1, resolved_options: {}, semantic_options: {}, provenance: {}, expires_at: '2099-01-01T00:00:00Z' },
      'encrypted-resolved-1'
    )).rejects.toThrow(/different printers/);
    await expect(client.jobs.createEncryptedResolved(
      { target_id: 'tgt_1', title: 'Private', content_type: 'pdf' },
      envelope,
      { digest: 'c'.repeat(64), printer_id: 'prt_1', capability_revision: 1, resolved_options: {}, semantic_options: {}, provenance: {}, expires_at: '2020-01-01T00:00:00Z' },
      'encrypted-expired-1'
    )).rejects.toThrow(/ticket has expired/);
    await expect(client.jobs.createEncryptedResolved(
      { target_id: 'tgt_1', title: 'Private', content_type: 'pdf' },
      envelope,
      { digest: 'c'.repeat(64), printer_id: 'prt_1', capability_revision: 1, resolved_options: {}, semantic_options: {}, provenance: {}, expires_at: 'not-a-date' },
      'encrypted-invalid-expiry-1'
    )).rejects.toThrow(/ticket has expired/);
    await expect(client.jobs.createEncryptedResolved(
      { target_id: 'tgt_1', title: 'Private', content_type: 'pdf' },
      envelope,
      { digest: 'c'.repeat(64), printer_id: 'prt_1', capability_revision: 1, resolved_options: {}, semantic_options: {}, provenance: {}, expires_at: '2099-02-30T00:00:00Z' },
      'encrypted-invalid-calendar-1'
    )).rejects.toThrow(/ticket has expired/);
  });
});
