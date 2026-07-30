import { error, redirect } from '@sveltejs/kit';
import type { RequestHandler } from './$types';
import {
  releaseObjectKey,
  signedReleaseAssetUrl,
  type ReleaseChannel
} from '$lib/server/release-origin';

export const GET: RequestHandler = async ({ params }) => {
  if (!releaseObjectKey(params.channel, params.asset)) {
    error(404, 'Release asset not found');
  }
  const destination = await signedReleaseAssetUrl(
    params.channel as ReleaseChannel,
    params.asset
  );
  if (!destination) {
    error(404, 'Release asset not published');
  }
  redirect(307, destination);
};
