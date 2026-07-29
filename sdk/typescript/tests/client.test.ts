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
});
