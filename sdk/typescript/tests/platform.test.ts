import { describe, expect, it, vi } from 'vitest';
import { SpoolPlatform } from '../src/index.js';
import type { PlatformAccount, Upload } from '../src/index.js';

const account: PlatformAccount = {
  id: 'wrk_customer_01',
  external_id: 'customer:42',
  name: 'Acme Labels',
  status: 'active',
  metadata: { plan: 'pro' },
  environments: {
    test: { id: 'env_customer_test', kind: 'test' },
    live: { id: 'env_customer_live', kind: 'live' }
  },
  created_at: '2026-07-29T00:00:00Z',
  updated_at: '2026-07-29T00:00:00Z'
};

function platform(fetcher: typeof fetch) {
  return new SpoolPlatform({
    platformKey: 'spl_platform_service_account',
    baseUrl: 'https://print.example.test/',
    fetch: fetcher,
    headers: {
      'x-sdk-version': 'test',
      'x-spool-workspace-id': 'must_be_stripped',
      'x-spool-environment-id': 'must_be_stripped'
    }
  });
}

describe('SpoolPlatform', () => {
  it('gets or creates an account and defaults its resources to Live', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(Response.json(account, { status: 201 }))
      .mockResolvedValueOnce(
        Response.json({ data: [], next_cursor: null, has_more: false })
      );

    const scoped = await platform(fetcher).accounts.getOrCreate('customer:42', {
      name: 'Acme Labels',
      metadata: { plan: 'pro' }
    });
    await scoped.printers.list();

    const [accountUrl, accountInit] = fetcher.mock.calls[0] ?? [];
    expect(String(accountUrl)).toBe(
      'https://print.example.test/v1/platform/accounts/customer%3A42'
    );
    expect(accountInit?.method).toBe('PUT');
    expect(JSON.parse(String(accountInit?.body))).toEqual({
      name: 'Acme Labels',
      metadata: { plan: 'pro' }
    });
    const accountHeaders = new Headers(accountInit?.headers);
    expect(accountHeaders.get('authorization')).toBe(
      'Bearer spl_platform_service_account'
    );
    expect(accountHeaders.get('x-spool-workspace-id')).toBeNull();
    expect(accountHeaders.get('x-spool-environment-id')).toBeNull();

    const [printerUrl, printerInit] = fetcher.mock.calls[1] ?? [];
    expect(String(printerUrl)).toBe('https://print.example.test/v1/printers');
    const printerHeaders = new Headers(printerInit?.headers);
    expect(printerHeaders.get('x-spool-workspace-id')).toBe('wrk_customer_01');
    expect(printerHeaders.get('x-spool-environment-id')).toBe('env_customer_live');
    expect(scoped.live).toBe(scoped);
    expect(scoped.externalId).toBe('customer:42');
    expect(scoped.jobs).toBeDefined();
    expect(scoped.nodes).toBeDefined();
    expect(scoped.uploads).toBeDefined();
    expect(scoped.apiKeys).toBeDefined();
  });

  it('keeps Test and Live resources separated without caller-managed headers', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(Response.json(account))
      .mockResolvedValueOnce(
        Response.json({ data: [], next_cursor: null, has_more: false })
      )
      .mockResolvedValueOnce(Response.json([]));
    const scoped = await platform(fetcher).accounts.retrieve('customer:42');

    await scoped.test.jobs.list();
    await scoped.live.nodes.list();

    const testHeaders = new Headers(fetcher.mock.calls[1]?.[1]?.headers);
    expect(testHeaders.get('x-spool-workspace-id')).toBe('wrk_customer_01');
    expect(testHeaders.get('x-spool-environment-id')).toBe('env_customer_test');
    const liveHeaders = new Headers(fetcher.mock.calls[2]?.[1]?.headers);
    expect(liveHeaders.get('x-spool-workspace-id')).toBe('wrk_customer_01');
    expect(liveHeaders.get('x-spool-environment-id')).toBe('env_customer_live');
  });

  it('lists scoped account clients and archives by external ID', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(Response.json([account]))
      .mockResolvedValueOnce(new Response(null, { status: 204 }));
    const spool = platform(fetcher);

    const accounts = await spool.accounts.list();
    await spool.accounts.archive('customer:42');

    expect(accounts[0]?.id).toBe('wrk_customer_01');
    expect(String(fetcher.mock.calls[0]?.[0])).toBe(
      'https://print.example.test/v1/platform/accounts'
    );
    const [archiveUrl, archiveInit] = fetcher.mock.calls[1] ?? [];
    expect(String(archiveUrl)).toBe(
      'https://print.example.test/v1/platform/accounts/customer%3A42'
    );
    expect(archiveInit?.method).toBe('DELETE');
  });

  it('keeps platform credentials and tenant headers off signed upload URLs', async () => {
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(Response.json(account))
      .mockResolvedValueOnce(new Response(null, { status: 200 }));
    const scoped = await platform(fetcher).accounts.retrieve('customer:42');

    await scoped.uploads.put(
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

    const headers = new Headers(fetcher.mock.calls[1]?.[1]?.headers);
    expect(headers.get('authorization')).toBeNull();
    expect(headers.get('x-spool-workspace-id')).toBeNull();
    expect(headers.get('x-spool-environment-id')).toBeNull();
    expect(headers.get('x-upload-token')).toBe('opaque');
  });

  it('uploads, verifies, and creates a PDF job in exactly three tenant calls', async () => {
    const sha256 = '315d429b7714cedb6ad04ac31240145257692630457f3c88253c5beceac76027';
    const pendingUpload: Upload & {
      upload_url: string;
      upload_method: 'PUT';
      upload_headers: Record<string, string>;
      requires_completion: false;
    } = {
      id: 'upl_pdf_01',
      object_key: 'hidden/object',
      media_type: 'application/pdf',
      expected_sha256: sha256,
      expected_bytes: 4,
      state: 'pending',
      expires_at: '2026-07-29T01:00:00Z',
      upload_url: '/v1/uploads/upl_pdf_01/content',
      upload_method: 'PUT',
      upload_headers: { 'content-type': 'application/pdf' },
      requires_completion: false
    };
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(Response.json(account))
      .mockResolvedValueOnce(Response.json(pendingUpload, { status: 201 }))
      .mockResolvedValueOnce(Response.json({ ...pendingUpload, state: 'complete' }))
      .mockResolvedValueOnce(
        Response.json(
          {
            id: 'job_01',
            printer_id: 'printer_01',
            title: 'Packing label',
            content_type: 'pdf',
            deliveries: 1,
            state: 'waiting_for_agent',
            created_at: '2026-07-29T00:00:00Z',
            expires_at: '2026-07-30T00:00:00Z'
          },
          { status: 201 }
        )
      );
    const scoped = await platform(fetcher).accounts.retrieve('customer:42');

    const job = await scoped.printPdf({
      targetId: 'target_shipping',
      title: 'Packing label',
      pdf: new Uint8Array([0x25, 0x50, 0x44, 0x46]),
      metadata: { order_id: 'order_42' },
      idempotencyKey: 'order_42-label-v1'
    });

    const tenantCalls = fetcher.mock.calls.slice(1);
    expect(tenantCalls).toHaveLength(3);
    expect(String(tenantCalls[0]?.[0])).toBe('https://print.example.test/v1/uploads');
    expect(JSON.parse(String(tenantCalls[0]?.[1]?.body))).toMatchObject({
      media_type: 'application/pdf',
      byte_length: 4,
      sha256
    });
    expect(String(tenantCalls[1]?.[0])).toBe(
      'https://print.example.test/v1/uploads/upl_pdf_01/content'
    );
    expect(String(tenantCalls[2]?.[0])).toBe('https://print.example.test/v1/jobs');
    expect(JSON.parse(String(tenantCalls[2]?.[1]?.body))).toMatchObject({
      target_id: 'target_shipping',
      title: 'Packing label',
      content_type: 'pdf',
      content: { type: 'upload', upload_id: 'upl_pdf_01' },
      metadata: { order_id: 'order_42' }
    });
    expect(new Headers(tenantCalls[2]?.[1]?.headers).get('idempotency-key')).toBe(
      'order_42-label-v1'
    );
    expect(job.id).toBe('job_01');
  });
});
