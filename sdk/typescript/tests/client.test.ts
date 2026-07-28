import { describe, expect, it, vi } from 'vitest';
import { SpoolClient } from '../src/index.js';

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
});
