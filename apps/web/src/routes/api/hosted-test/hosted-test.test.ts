import { beforeEach, describe, expect, it, vi } from 'vitest';

const { createJob, agentRequest } = vi.hoisted(() => ({
  createJob: vi.fn(),
  agentRequest: vi.fn()
}));

vi.mock('$lib/server/dashboard-data', () => ({
  dashboardMode: () => 'live',
  dashboardSdk: () => ({ jobs: { create: createJob } })
}));
vi.mock('$lib/server/local-agent', () => ({
  LocalAgentConfigurationError: class LocalAgentConfigurationError extends Error {},
  createA4TestPdf: () => 'pdf-base64',
  localAgentError: () =>
    Response.json({ code: 'local_agent_unavailable', message: 'Local agent unavailable.' }, { status: 502 }),
  localAgentRequest: agentRequest,
  relayLocalAgent: (response: Response) => response
}));

import { POST } from './+server';

function event(body: Record<string, unknown>) {
  return {
    request: new Request('https://spool.test/api/hosted-test', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body)
    }),
    fetch: vi.fn(),
    url: new URL('https://spool.test/api/hosted-test'),
    locals: {}
  } as never;
}

describe('hosted A4 test', () => {
  beforeEach(() => {
    createJob.mockReset();
    agentRequest.mockReset();
  });

  it('resolves immutable profile settings server-side and registers a canonical hosted job', async () => {
    agentRequest.mockResolvedValue(
      Response.json([
        {
          profile_id: 'prof_a4',
          revision: 4,
          name: 'A4 shipping',
          is_default: true,
          options: {
            paper: 'A4',
            color: false,
            duplex: 'one-sided',
            native_options: { InputSlot: 'Tray2' }
          }
        }
      ])
    );
    createJob.mockResolvedValue({ id: 'job_01', state: 'registered' });

    const response = await POST(
      event({ printer_id: 'prt_01', profile_id: 'prof_a4', confirmed: true })
    );

    expect(response.status).toBe(201);
    expect(createJob).toHaveBeenCalledWith(
      expect.objectContaining({
        printer_id: 'prt_01',
        content_type: 'pdf',
        content: { type: 'base64', data: 'pdf-base64' },
        options: expect.objectContaining({
          paper: 'A4',
          native_options: { InputSlot: 'Tray2' }
        }),
        metadata: expect.objectContaining({
          profile_id: 'prof_a4',
          profile_revision: '4'
        })
      }),
      expect.stringMatching(/^a4-test-/)
    );
    expect(await response.json()).toEqual({ job_id: 'job_01', state: 'registered' });
  });

  it('rejects a profile that explicitly selects non-A4 stock', async () => {
    agentRequest.mockResolvedValue(
      Response.json([
        {
          profile_id: 'prof_label',
          revision: 1,
          name: 'Labels',
          is_default: true,
          options: { paper: '4x6', color: false }
        }
      ])
    );

    const response = await POST(
      event({ printer_id: 'prt_01', profile_id: 'prof_label', confirmed: true })
    );

    expect(response.status).toBe(409);
    expect(await response.json()).toMatchObject({ code: 'a4_profile_required' });
    expect(createJob).not.toHaveBeenCalled();
  });

  it('preserves a local profile lookup failure', async () => {
    agentRequest.mockResolvedValue(
      Response.json(
        { code: 'printer_not_found', message: 'The selected printer was not found.' },
        { status: 422 }
      )
    );

    const response = await POST(
      event({ printer_id: 'prt_missing', profile_id: 'prof_a4', confirmed: true })
    );

    expect(response.status).toBe(422);
    expect(await response.json()).toMatchObject({ code: 'printer_not_found' });
    expect(createJob).not.toHaveBeenCalled();
  });

  it('returns 400 for malformed JSON', async () => {
    const malformedEvent = {
      request: new Request('https://spool.test/api/hosted-test', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: '{'
      }),
      fetch: vi.fn(),
      url: new URL('https://spool.test/api/hosted-test'),
      locals: {}
    } as never;

    const response = await POST(malformedEvent);

    expect(response.status).toBe(400);
    expect(await response.json()).toMatchObject({ code: 'invalid_request' });
    expect(agentRequest).not.toHaveBeenCalled();
  });
});
