import { env } from '$env/dynamic/private';
import { productEnvironmentValue } from './product-env';
import { readFile } from 'node:fs/promises';

export interface LocalAgentStatus {
  agent_id: string | null;
  workspace_name: string | null;
  version: string;
  connection: 'local_only' | 'connected' | 'connecting' | 'offline' | 'degraded';
  queued_jobs: number;
  active_jobs: number;
  printer_warnings: number;
  paused: boolean;
}

export interface LocalPrinter {
  printer_id: string;
  name: string;
  state: string;
  is_default: boolean;
}

export class LocalAgentConfigurationError extends Error {
  readonly status = 503;

  constructor(message: string) {
    super(message);
    this.name = 'LocalAgentConfigurationError';
  }
}

function configuredBaseUrl(): URL {
  const rawUrl = productEnvironmentValue(env, 'PIQAE_LOCAL_AGENT_URL')?.trim();
  const tokenFile = productEnvironmentValue(env, 'PIQAE_LOCAL_AGENT_TOKEN_FILE')?.trim();

  if (!rawUrl && !tokenFile) {
    throw new LocalAgentConfigurationError(
      'Local agent access is disabled. Set PIQAE_LOCAL_AGENT_URL and PIQAE_LOCAL_AGENT_TOKEN_FILE on the web server.'
    );
  }
  if (!rawUrl || !tokenFile) {
    throw new LocalAgentConfigurationError(
      'Local agent access requires both PIQAE_LOCAL_AGENT_URL and PIQAE_LOCAL_AGENT_TOKEN_FILE.'
    );
  }

  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    throw new LocalAgentConfigurationError('PIQAE_LOCAL_AGENT_URL is not a valid URL.');
  }
  const hostname = url.hostname.replace(/^\[|\]$/g, '').toLowerCase();
  if (
    !['http:', 'https:'].includes(url.protocol) ||
    !['localhost', '127.0.0.1', '::1'].includes(hostname) ||
    url.username ||
    url.password ||
    url.search ||
    url.hash
  ) {
    throw new LocalAgentConfigurationError(
      'PIQAE_LOCAL_AGENT_URL must be an HTTP(S) loopback URL without credentials, query, or fragment.'
    );
  }
  if (url.pathname !== '/' && url.pathname !== '') {
    throw new LocalAgentConfigurationError('PIQAE_LOCAL_AGENT_URL must not include a path.');
  }
  return url;
}

async function localToken(): Promise<string> {
  const tokenFile = productEnvironmentValue(env, 'PIQAE_LOCAL_AGENT_TOKEN_FILE')?.trim();
  if (!tokenFile) {
    throw new LocalAgentConfigurationError(
      'Local agent access requires PIQAE_LOCAL_AGENT_TOKEN_FILE.'
    );
  }
  let token: string;
  try {
    token = (await readFile(tokenFile, 'utf8')).trim();
  } catch {
    throw new LocalAgentConfigurationError('The local agent token file could not be read.');
  }
  if (!token) {
    throw new LocalAgentConfigurationError('The local agent token file is empty.');
  }
  return token;
}

export async function localAgentRequest(
  fetcher: typeof fetch,
  path: string,
  init: RequestInit = {}
): Promise<Response> {
  if (
    !path.startsWith('/') ||
    path.startsWith('//') ||
    path.includes('\\') ||
    /[\u0000-\u001f\u007f]/.test(path)
  ) {
    throw new TypeError('Local agent paths must be absolute pathnames.');
  }
  const baseUrl = configuredBaseUrl();
  const targetUrl = new URL(path, baseUrl);
  if (targetUrl.origin !== baseUrl.origin) {
    throw new TypeError('Local agent paths must remain on the configured loopback origin.');
  }
  const token = await localToken();
  const headers = new Headers(init.headers);
  headers.set('authorization', `Bearer ${token}`);
  headers.set('accept', 'application/json');
  if (init.body) headers.set('content-type', 'application/json');

  return fetcher(targetUrl, {
    ...init,
    headers,
    cache: 'no-store'
  });
}

export function localAgentError(error: unknown): Response {
  const configuration = error instanceof LocalAgentConfigurationError;
  return Response.json(
    {
      code: configuration ? 'local_agent_not_configured' : 'local_agent_unavailable',
      message: configuration
        ? error.message
        : 'The local agent did not respond. Confirm that Piqae is running on this Mac.'
    },
    {
      status: configuration ? error.status : 502,
      headers: { 'cache-control': 'no-store, private' }
    }
  );
}

export async function relayLocalAgent(response: Response): Promise<Response> {
  const body = response.status === 204 ? null : await response.arrayBuffer();
  const headers = new Headers({
    'cache-control': 'no-store, private'
  });
  const contentType = response.headers.get('content-type');
  if (contentType) headers.set('content-type', contentType);
  return new Response(body, { status: response.status, headers });
}

export function createA4TestPdf(): string {
  const stream =
    'BT\n/F1 22 Tf\n72 758 Td\n(Piqae A4 printer test) Tj\n0 -34 Td\n/F1 11 Tf\n(Local driver and queue are working.) Tj\nET\n';
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>',
    `<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}endstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>'
  ];
  let pdf = '%PDF-1.4\n';
  const offsets = [0];
  for (const [index, object] of objects.entries()) {
    offsets.push(Buffer.byteLength(pdf));
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
  }
  const xref = Buffer.byteLength(pdf);
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  pdf += offsets
    .slice(1)
    .map((offset) => `${String(offset).padStart(10, '0')} 00000 n \n`)
    .join('');
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(pdf).toString('base64');
}
