import type { RequestHandler } from './$types';
import { publishedLicensedFont } from '$lib/server/licensed-font-origin';

export const GET: RequestHandler = async ({ params }) => {
  const font = await publishedLicensedFont(params.asset);
  if (!font) {
    return new Response('Not found', {
      status: 404,
      headers: {
        'cache-control': 'no-store',
        'content-type': 'text/plain; charset=utf-8',
        'x-content-type-options': 'nosniff'
      }
    });
  }

  const body = Uint8Array.from(font.bytes).buffer;
  return new Response(body, {
    headers: {
      'cache-control': 'public, max-age=31536000, immutable',
      'content-type': 'font/woff2',
      'x-content-type-options': 'nosniff'
    }
  });
};
