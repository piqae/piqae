import { describe, expect, it, vi } from 'vitest';
import { createLiveApi } from './api';

const uncertainJob = (id: string, since: string) => ({
  id,
  printer_id: 'prt_01',
  title: id,
  content_type: 'pdf',
  deliveries: 1,
  state: 'delivery_uncertain',
  created_at: '2026-08-25T10:00:00.000Z',
  expires_at: '2026-08-26T10:00:00.000Z',
  delivery_uncertain_since: since
});

describe('live dashboard overview', () => {
  it('paginates the server-filtered uncertain set and maps its transition timestamp', async () => {
    const first = Array.from({ length: 100 }, (_, index) =>
      uncertainJob(`job_${index}`, '2026-08-25T09:00:00.000Z')
    );
    const fetcher = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === '/v1/agents') return Response.json([]);
      if (url.pathname === '/v1/printers') {
        return Response.json({ data: [], next_cursor: null, has_more: false });
      }
      if (url.pathname === '/v1/jobs/job_detail') {
        return Response.json(uncertainJob('job_detail', '2026-08-25T08:00:00.000Z'));
      }
      if (url.pathname === '/v1/jobs' && url.searchParams.get('state') === 'delivery_uncertain') {
        return url.searchParams.get('after') === 'page_2'
          ? Response.json({
              data: [uncertainJob('job_100', '2026-08-25T07:00:00.000Z')],
              next_cursor: null,
              has_more: false
            })
          : Response.json({ data: first, next_cursor: 'page_2', has_more: true });
      }
      if (url.pathname === '/v1/jobs') {
        return Response.json({ data: [], next_cursor: null, has_more: false });
      }
      throw new Error(`Unexpected request: ${url}`);
    });
    const api = createLiveApi(fetcher as typeof fetch, 'https://api.example.test');

    await expect(api.overview()).resolves.toMatchObject({
      jobs: {
        uncertain: 101,
        oldestUncertainSince: '2026-08-25T07:00:00.000Z'
      }
    });
    await expect(api.job('job_detail')).resolves.toMatchObject({
      deliveryUncertainSince: '2026-08-25T08:00:00.000Z'
    });
    expect(
      fetcher.mock.calls.some(([input]) => new URL(String(input)).searchParams.get('after') === 'page_2')
    ).toBe(true);
    expect(
      fetcher.mock.calls.filter(
        ([input]) =>
          new URL(String(input)).pathname === '/v1/jobs' &&
          new URL(String(input)).searchParams.get('state') === 'delivery_uncertain'
      )
    ).toHaveLength(2);
  });

  it('fails closed when a paginated response claims another page without a cursor', async () => {
    const fetcher = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === '/v1/agents') return Response.json([]);
      if (url.pathname === '/v1/printers' || url.pathname === '/v1/jobs') {
        const missingCursor =
          url.pathname === '/v1/jobs' &&
          url.searchParams.get('state') === 'delivery_uncertain';
        return Response.json({ data: [], next_cursor: null, has_more: missingCursor });
      }
      throw new Error(`Unexpected request: ${url}`);
    });

    await expect(
      createLiveApi(fetcher as typeof fetch, 'https://api.example.test').overview()
    ).rejects.toThrow('reported more results without a cursor');
  });

  it('bounds pagination even when every response supplies a unique cursor', async () => {
    let uncertainPage = 0;
    const fetcher = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === '/v1/agents') return Response.json([]);
      if (url.pathname === '/v1/printers') {
        return Response.json({ data: [], next_cursor: null, has_more: false });
      }
      if (url.pathname === '/v1/jobs' && url.searchParams.get('state') === 'delivery_uncertain') {
        uncertainPage += 1;
        return Response.json({
          data: [],
          next_cursor: `page_${uncertainPage + 1}`,
          has_more: true
        });
      }
      if (url.pathname === '/v1/jobs') {
        return Response.json({ data: [], next_cursor: null, has_more: false });
      }
      throw new Error(`Unexpected request: ${url}`);
    });

    await expect(
      createLiveApi(fetcher as typeof fetch, 'https://api.example.test').overview()
    ).rejects.toThrow('exceeded its pagination bound');
    expect(uncertainPage).toBe(100);
  });
});
