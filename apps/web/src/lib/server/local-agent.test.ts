import { beforeEach, describe, expect, it, vi } from 'vitest';

const { environment, readFile } = vi.hoisted(() => ({
  environment: {} as Record<string, string>,
  readFile: vi.fn()
}));

vi.mock('$env/dynamic/private', () => ({ env: environment }));
vi.mock('node:fs/promises', () => ({ default: { readFile }, readFile }));

import {
  LocalAgentConfigurationError,
  createA4TestPdf,
  localAgentRequest
} from './local-agent';

describe('local agent server adapter', () => {
  beforeEach(() => {
    for (const key of Object.keys(environment)) delete environment[key];
    readFile.mockReset();
  });

  it('requires the URL and token file as an explicit pair', async () => {
    environment.SPOOL_LOCAL_AGENT_URL = 'http://127.0.0.1:17890';

    await expect(localAgentRequest(vi.fn(), '/v1/local/status')).rejects.toThrow(
      LocalAgentConfigurationError
    );
    expect(readFile).not.toHaveBeenCalled();
  });

  it('rejects non-loopback agent URLs before reading credentials', async () => {
    environment.SPOOL_LOCAL_AGENT_URL = 'https://agent.example.com';
    environment.SPOOL_LOCAL_AGENT_TOKEN_FILE = '/tmp/local.token';

    await expect(localAgentRequest(vi.fn(), '/v1/local/status')).rejects.toThrow(
      /loopback URL/
    );
    expect(readFile).not.toHaveBeenCalled();
  });

  it.each([
    '//attacker.example/steal',
    '/\\attacker.example/steal',
    '/v1/local/status\\..\\..\\steal',
    '/v1/local/status\u0000'
  ])('rejects path escape %s before reading credentials', async (path) => {
    environment.SPOOL_LOCAL_AGENT_URL = 'http://127.0.0.1:17890';
    environment.SPOOL_LOCAL_AGENT_TOKEN_FILE = '/tmp/local.token';

    await expect(localAgentRequest(vi.fn(), path)).rejects.toThrow(/absolute pathnames/);
    expect(readFile).not.toHaveBeenCalled();
  });

  it('reads the token server-side and forwards it only in the upstream header', async () => {
    environment.SPOOL_LOCAL_AGENT_URL = 'http://localhost:17890';
    environment.SPOOL_LOCAL_AGENT_TOKEN_FILE = '/var/run/spool/local.token';
    readFile.mockResolvedValue('session-secret\n');
    const fetcher = vi.fn<typeof fetch>().mockResolvedValue(
      Response.json({ version: '0.1.0', connection: 'connected' })
    );

    const response = await localAgentRequest(fetcher, '/v1/local/status');
    const [url, init] = fetcher.mock.calls[0] ?? [];
    const headers = new Headers(init?.headers);

    expect(String(url)).toBe('http://localhost:17890/v1/local/status');
    expect(readFile).toHaveBeenCalledWith('/var/run/spool/local.token', 'utf8');
    expect(headers.get('authorization')).toBe('Bearer session-secret');
    expect(await response.text()).not.toContain('session-secret');
  });

  it('builds a real A4 PDF for local test jobs', () => {
    const bytes = Buffer.from(createA4TestPdf(), 'base64');
    expect(bytes.subarray(0, 8).toString()).toBe('%PDF-1.4');
    expect(bytes.toString()).toContain('/MediaBox [0 0 595 842]');
    expect(bytes.toString()).toContain('Spool A4 printer test');
  });
});
