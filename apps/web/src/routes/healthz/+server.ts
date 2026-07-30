import { env } from '$env/dynamic/public';
import { productEnvironmentValue } from '$lib/server/product-env';

export function GET(): Response {
  return Response.json(
    {
      status: 'ok',
      service: 'piqae-web',
      version:
        productEnvironmentValue(env, 'PUBLIC_PIQAE_VERSION')?.trim() || '0.1.0'
    },
    {
      headers: {
        'cache-control': 'no-store'
      }
    }
  );
}
