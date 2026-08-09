import type { PageServerLoad } from './$types';
import { dashboardMeta } from '$lib/server/dashboard-data';
import {
  detectClient,
  combineReleaseManifests,
  loadReleaseManifest,
  recommendedArtifact
} from '$lib/server/release-manifest';
import { publishedReleaseManifest } from '$lib/server/release-origin';

export const load: PageServerLoad = async (event) => {
  const [stable, preview] = await Promise.all([
    publishedReleaseManifest('stable'),
    publishedReleaseManifest('preview')
  ]);
  const manifest = combineReleaseManifests(stable, preview) ?? loadReleaseManifest();
  const detected = detectClient(event.request.headers);
  return {
    meta: await dashboardMeta(event),
    manifest,
    detected,
    recommendedArtifactId: recommendedArtifact(manifest, detected)
  };
};
