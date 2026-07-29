import { error } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { dashboardConnection, dashboardMode } from '$lib/server/dashboard-data';

export const GET: RequestHandler = async (event) => {
  if (dashboardMode() !== 'live') error(404, 'Live events are disabled in demo mode');

  const { baseUrl, bearerToken } = dashboardConnection(event);
  const upstreamUrl = new URL('/v1/events/stream', `${baseUrl}/`);
  const lastEventId = event.request.headers.get('last-event-id');
  const headers = new Headers({
    accept: 'text/event-stream',
    authorization: `Bearer ${bearerToken}`,
    'cache-control': 'no-cache'
  });
  if (lastEventId) headers.set('last-event-id', lastEventId);

  const upstream = await event.fetch(upstreamUrl, {
    method: 'GET',
    headers,
    signal: event.request.signal
  });
  if (!upstream.ok || !upstream.body) {
    error(upstream.status || 502, 'Control-plane event stream is unavailable');
  }

  return new Response(upstream.body, {
    status: 200,
    headers: {
      'content-type': 'text/event-stream',
      'cache-control': 'no-cache, no-transform',
      connection: 'keep-alive',
      'x-accel-buffering': 'no'
    }
  });
};
