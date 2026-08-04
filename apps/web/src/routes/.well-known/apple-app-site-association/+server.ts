import { env } from '$env/dynamic/private';
import type { RequestHandler } from './$types';

const TEAM_ID = /^[A-Z0-9]{10}$/;

export const GET: RequestHandler = () => {
  const teamID = env.APPLE_TEAM_ID?.trim() ?? '';
  if (!TEAM_ID.test(teamID)) {
    return new Response('Universal Links are not configured.', {
      status: 503,
      headers: { 'cache-control': 'no-store', 'content-type': 'text/plain; charset=utf-8' }
    });
  }
  return Response.json(
    {
      applinks: {
        details: [
          {
            appIDs: [`${teamID}.com.c4coffee.spool.menu`],
            components: [{ '/': '/connect', comment: 'Piqae node connector consent handoff' }]
          }
        ]
      }
    },
    { headers: { 'cache-control': 'public, max-age=3600' } }
  );
};
