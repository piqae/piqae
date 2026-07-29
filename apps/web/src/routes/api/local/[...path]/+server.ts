import type { RequestHandler } from './$types';
import {
  localAgentError,
  localAgentRequest,
  relayLocalAgent
} from '$lib/server/local-agent';

function notFound(): Response {
  return Response.json(
    { code: 'not_found', message: 'Unknown local agent operation.' },
    { status: 404 }
  );
}

export function _pathFor(method: string, path: string): string | null {
  const segments = path.split('/').filter(Boolean);
  if (segments.some((segment) => segment.length > 512)) return null;
  if (method === 'GET' && segments.length === 1 && ['status', 'printers'].includes(segments[0] ?? '')) {
    return `/v1/local/${segments[0]}`;
  }
  if (method === 'POST' && segments.length === 1 && ['pause', 'resume'].includes(segments[0] ?? '')) {
    return `/v1/local/${segments[0]}`;
  }
  if (
    method === 'POST' &&
    segments[0] === 'profiles' &&
    segments[1] &&
    segments[2] === 'validate' &&
    segments.length === 3
  ) {
    return `/v1/local/profiles/${encodeURIComponent(segments[1])}/validate`;
  }
  if (
    segments[0] === 'printers' &&
    segments[1] &&
    segments.length === 3 &&
    ((method === 'GET' && ['queue', 'profiles'].includes(segments[2] ?? '')) ||
      (method === 'POST' && segments[2] === 'test-page') ||
      (method === 'PUT' && segments[2] === 'exposure'))
  ) {
    return `/v1/local/printers/${encodeURIComponent(segments[1])}/${segments[2]}`;
  }
  if (
    segments[0] === 'printers' &&
    segments[1] &&
    segments[2] === 'profiles' &&
    segments[3] &&
    segments.length === 4 &&
    method === 'DELETE'
  ) {
    return `/v1/local/printers/${encodeURIComponent(segments[1])}/profiles/${encodeURIComponent(segments[3])}`;
  }
  return null;
}

const maximumBodyBytes = 65_536;

class RequestBodyTooLarge extends Error {}

async function requestBody(request: Request): Promise<ArrayBuffer | undefined> {
  const declaredLength = Number(request.headers.get('content-length'));
  if (Number.isFinite(declaredLength) && declaredLength > maximumBodyBytes) {
    throw new RequestBodyTooLarge();
  }
  const reader = request.body?.getReader();
  if (!reader) return undefined;
  const chunks: Uint8Array[] = [];
  let length = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    length += value.byteLength;
    if (length > maximumBodyBytes) {
      await reader.cancel();
      throw new RequestBodyTooLarge();
    }
    chunks.push(value);
  }
  if (length === 0) return undefined;
  const body = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return body.buffer;
}

function proxyError(error: unknown): Response {
  if (error instanceof RequestBodyTooLarge) {
    return Response.json(
      {
        code: 'request_too_large',
        message: 'Local management requests are limited to 64 KiB.'
      },
      { status: 413, headers: { 'cache-control': 'no-store, private' } }
    );
  }
  return localAgentError(error);
}

export const GET: RequestHandler = async ({ params, fetch }) => {
  const path = _pathFor('GET', params.path);
  if (!path) return notFound();
  try {
    return await relayLocalAgent(await localAgentRequest(fetch, path));
  } catch (error) {
    return localAgentError(error);
  }
};

export const POST: RequestHandler = async ({ params, fetch, request }) => {
  const path = _pathFor('POST', params.path);
  if (!path) return notFound();
  try {
    return await relayLocalAgent(
      await localAgentRequest(fetch, path, { method: 'POST', body: await requestBody(request) })
    );
  } catch (error) {
    return proxyError(error);
  }
};

export const PUT: RequestHandler = async ({ params, fetch, request }) => {
  const path = _pathFor('PUT', params.path);
  if (!path) return notFound();
  try {
    return await relayLocalAgent(
      await localAgentRequest(fetch, path, { method: 'PUT', body: await requestBody(request) })
    );
  } catch (error) {
    return proxyError(error);
  }
};

export const DELETE: RequestHandler = async ({ params, fetch, request, url }) => {
  const basePath = _pathFor('DELETE', params.path);
  if (!basePath) return notFound();
  const expectedRevision = url.searchParams.get('expected_revision');
  const path =
    expectedRevision && /^\d+$/.test(expectedRevision)
      ? `${basePath}?expected_revision=${expectedRevision}`
      : basePath;
  try {
    return await relayLocalAgent(
      await localAgentRequest(fetch, path, { method: 'DELETE', body: await requestBody(request) })
    );
  } catch (error) {
    return proxyError(error);
  }
};
