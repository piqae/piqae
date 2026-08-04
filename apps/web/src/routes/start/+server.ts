import { redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import { buildAttribution, type MarketingAttribution } from '$lib/marketing/attribution';

export const GET: RequestHandler = ({ url, cookies, request }) => {
  let existing: MarketingAttribution | undefined;
  const stored = cookies.get('piqae_attribution');
  if (stored) {
    try {
      existing = JSON.parse(Buffer.from(stored, 'base64url').toString('utf8')) as MarketingAttribution;
    } catch {
      existing = undefined;
    }
  }
  const attribution = buildAttribution(url, existing, request.headers.get('referer') ?? undefined);
  cookies.set('piqae_attribution', Buffer.from(JSON.stringify(attribution), 'utf8').toString('base64url'), {
    path: '/',
    httpOnly: true,
    sameSite: 'lax',
    secure: url.protocol === 'https:',
    maxAge: 60 * 60 * 24 * 30
  });
  const returnTo = attribution.plan === 'free' ? '/dashboard' : '/dashboard/settings#billing';
  redirect(303, `/auth/login?return_to=${encodeURIComponent(returnTo)}`);
};
