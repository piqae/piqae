import type { PageServerLoad } from './$types';
import { dashboardMeta } from '$lib/server/dashboard-data';
import {
  detectClient,
  loadReleaseManifest,
  recommendedArtifact
} from '$lib/server/release-manifest';
import { publishedReleaseManifest } from '$lib/server/release-origin';

export const load: PageServerLoad = async (event) => {
  const manifest = (await publishedReleaseManifest()) ?? loadReleaseManifest();
  const detected = detectClient(event.request.headers);
  return {
    meta: await dashboardMeta(event),
    manifest,
    detected,
    recommendedArtifactId: recommendedArtifact(manifest, detected)
  };
};
