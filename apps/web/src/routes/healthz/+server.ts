import { env } from '$env/dynamic/public';

export function GET(): Response {
  return Response.json(
    {
      status: 'ok',
      service: 'piqae-web',
      version: env.PUBLIC_SPOOL_VERSION?.trim() || '0.1.0'
    },
    {
      headers: {
        'cache-control': 'no-store'
      }
    }
  );
}
