import type { LoaderFunctionArgs } from "react-router";

const PACKAGE_VERSION = "0.2.0";

function releaseRevision(): string {
  const revision =
    process.env.PIQAE_RELEASE_SHA ?? process.env.RAILWAY_GIT_COMMIT_SHA ?? "";
  return /^[0-9a-f]{40}$/i.test(revision) ? revision.toLowerCase() : "unknown";
}

export function loader(_args: LoaderFunctionArgs): Response {
  return Response.json(
    {
      status: "ok",
      service: "piqae-shopify",
      version: PACKAGE_VERSION,
      revision: releaseRevision(),
    },
    { headers: { "cache-control": "no-store" } },
  );
}
