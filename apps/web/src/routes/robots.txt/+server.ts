import { env } from '$env/dynamic/public';
import type { RequestHandler } from './$types';

export const GET: RequestHandler = () => {
  const origin = env.PUBLIC_SITE_URL?.replace(/\/$/, '');
  const indexable = env.PUBLIC_MARKETING_INDEXABLE === 'true' && Boolean(origin);
  const body = indexable
    ? `User-agent: *\nAllow: /\nDisallow: /api/\nDisallow: /auth/\nDisallow: /dashboard/\nDisallow: /login\nDisallow: /pair/\nDisallow: /compare/qz-tray\nDisallow: /compare/ezeep\nSitemap: ${origin}/sitemap.xml\n`
    : 'User-agent: *\nDisallow: /\n';
  return new Response(body, { headers: { 'content-type': 'text/plain; charset=utf-8' } });
};

