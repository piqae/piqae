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

function pathFor(method: string, path: string): string | null {
  const segments = path.split('/').filter(Boolean);
  if (segments.some((segment) => segment.length > 512)) return null;
  if (method === 'GET' && segments.length === 1 && ['status', 'printers'].includes(segments[0] ?? '')) {
    return `/v1/local/${segments[0]}`;
  }
  if (method === 'POST' && segments.length === 1 && ['pause', 'resume'].includes(segments[0] ?? '')) {
    return `/v1/local/${segments[0]}`;
  }
  if (
    segments[0] === 'printers' &&
    segments[1] &&
    segments.length === 3 &&
    ((method === 'GET' && ['queue', 'profiles'].includes(segments[2] ?? '')) ||
      (method === 'POST' && ['profiles', 'test-page'].includes(segments[2] ?? '')) ||
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
    ['PUT', 'DELETE'].includes(method)
  ) {
    return `/v1/local/printers/${encodeURIComponent(segments[1])}/profiles/${encodeURIComponent(segments[3])}`;
  }
  return null;
}

async function requestBody(request: Request): Promise<string | undefined> {
  const body = await request.text();
  if (body.length > 65_536) throw new TypeError('Local management requests are limited to 64 KiB.');
  return body || undefined;
}

export const GET: RequestHandler = async ({ params, fetch }) => {
  const path = pathFor('GET', params.path);
  if (!path) return notFound();
  try {
    return await relayLocalAgent(await localAgentRequest(fetch, path));
  } catch (error) {
    return localAgentError(error);
  }
};

export const POST: RequestHandler = async ({ params, fetch, request }) => {
  const path = pathFor('POST', params.path);
  if (!path) return notFound();
  try {
    return await relayLocalAgent(
      await localAgentRequest(fetch, path, { method: 'POST', body: await requestBody(request) })
    );
  } catch (error) {
    return localAgentError(error);
  }
};

export const PUT: RequestHandler = async ({ params, fetch, request }) => {
  const path = pathFor('PUT', params.path);
  if (!path) return notFound();
  try {
    return await relayLocalAgent(
      await localAgentRequest(fetch, path, { method: 'PUT', body: await requestBody(request) })
    );
  } catch (error) {
    return localAgentError(error);
  }
};

export const DELETE: RequestHandler = async ({ params, fetch, request, url }) => {
  const basePath = pathFor('DELETE', params.path);
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
    return localAgentError(error);
  }
};
