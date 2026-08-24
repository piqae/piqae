import { env } from '$env/dynamic/public';
import { env as privateEnv } from '$env/dynamic/private';
import { productEnvironmentValue } from '$lib/server/product-env';

/**
 * The commit this build was produced from.
 *
 * A health probe that reports only a package version cannot tell a stale
 * container from the reviewed one, because the version rarely changes between
 * commits. Reporting the commit lets a post-deploy gate assert that the live
 * build is the build that was reviewed. Anything that is not a full commit
 * hash becomes `unknown` rather than being echoed back, so the gate can never
 * be satisfied by an arbitrary string.
 */
function releaseRevision(): string {
  const revision =
    productEnvironmentValue(privateEnv, 'PIQAE_RELEASE_SHA') ??
    privateEnv.RAILWAY_GIT_COMMIT_SHA ??
    '';
  return /^[0-9a-f]{40}$/i.test(revision.trim())
    ? revision.trim().toLowerCase()
    : 'unknown';
}

export function GET(): Response {
  return Response.json(
    {
      status: 'ok',
      service: 'piqae-web',
      version:
        productEnvironmentValue(env, 'PUBLIC_PIQAE_VERSION')?.trim() || '0.1.0',
      revision: releaseRevision()
    },
    {
      headers: {
        'cache-control': 'no-store'
      }
    }
  );
}
