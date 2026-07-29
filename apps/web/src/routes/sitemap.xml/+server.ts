import { env } from '$env/dynamic/public';
import type { RequestHandler } from './$types';
import { printNodePricingReviewDueAt } from '$lib/marketing/calculator';

const stableRoutes = [
  '/',
  '/how-it-works',
  '/downloads',
  '/pricing',
  '/about',
  '/compare',
  '/open-source',
  '/security',
  '/docs'
];
const reviewedPrintNodeRoutes = [
  '/compare/printnode',
  '/alternatives/printnode',
  '/migrate/printnode',
  '/tools/printnode-cost-calculator'
];

export const GET: RequestHandler = () => {
  const origin = env.PUBLIC_SITE_URL?.replace(/\/$/, '');
  const empty = '<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9"></urlset>';
  if (!origin || env.PUBLIC_MARKETING_INDEXABLE !== 'true') {
    return new Response(empty, { headers: { 'content-type': 'application/xml; charset=utf-8' } });
  }
  const claimsCurrent = new Date() <= new Date(`${printNodePricingReviewDueAt}T23:59:59Z`);
  const routes = claimsCurrent ? [...stableRoutes, ...reviewedPrintNodeRoutes] : stableRoutes;
  const urls = routes.map((path) => `<url><loc>${origin}${path}</loc></url>`).join('');
  return new Response(
    `<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">${urls}</urlset>`,
    { headers: { 'content-type': 'application/xml; charset=utf-8' } }
  );
};
