import type { PageServerLoad } from './$types';
import { dashboardMeta } from '$lib/server/dashboard-data';
import {
  detectClient,
  loadReleaseManifest,
  recommendedArtifact
} from '$lib/server/release-manifest';

export const load: PageServerLoad = async (event) => {
  const manifest = loadReleaseManifest();
  const detected = detectClient(event.request.headers);
  return {
    meta: await dashboardMeta(event),
    manifest,
    detected,
    recommendedArtifactId: recommendedArtifact(manifest, detected)
  };
};
