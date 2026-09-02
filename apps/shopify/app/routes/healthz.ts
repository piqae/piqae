import type { LoaderFunctionArgs } from "react-router";
import { readFileSync } from "node:fs";

const PACKAGE_VERSION = "0.2.0";
const RELEASE_REVISION = /^[0-9a-f]{40}$/i;

function normalizeRevision(value: string | undefined): string {
  const revision = value?.trim() ?? "";
  return RELEASE_REVISION.test(revision) ? revision.toLowerCase() : "unknown";
}

function bakedReleaseRevision(): string {
  try {
    return normalizeRevision(readFileSync("/app/.piqae-release-sha", "utf8"));
  } catch {
    return "unknown";
  }
}

function releaseRevision(): string {
  // Production evidence must describe the immutable image that is serving the
  // request. Railway service variables can outlive or be changed independently
  // of an active deployment, so they are only a local/test fallback.
  if (process.env.NODE_ENV === "production") return bakedReleaseRevision();
  const revision =
    process.env.PIQAE_RELEASE_SHA ?? process.env.RAILWAY_GIT_COMMIT_SHA ?? "";
  return normalizeRevision(revision);
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
