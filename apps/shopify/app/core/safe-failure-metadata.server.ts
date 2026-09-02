import { PiqaeError } from "@piqae/sdk";

import { DocumentRenderFailedError } from "./document-render-errors";
import { ShopifyAdminGraphqlError } from "./orders.server";

const SAFE_ERROR_NAMES = new Set([
  "AbortError",
  "Error",
  "ShopifySessionRecoveryError",
  "TypeError",
]);

type ShopifyHttpFailure = {
  response?: {
    code?: unknown;
  };
};

type PostgreSqlFailure = {
  name?: unknown;
  code?: unknown;
};

/** Operational metadata only: never include request bodies or error messages. */
export function safeFailureMetadata(error: unknown) {
  if (error instanceof DocumentRenderFailedError)
    return { renderFailureCode: error.failureCode };
  if (error instanceof PiqaeError)
    return {
      upstreamCode: error.code,
      upstreamStatus: error.status,
      upstreamRequestId: error.requestId,
      retryable: error.retryable,
    };
  if (error instanceof ShopifyAdminGraphqlError)
    return { upstream: "shopify_admin", failureKind: "graphql_query" };
  if (error && typeof error === "object") {
    const status = (error as ShopifyHttpFailure).response?.code;
    if (typeof status === "number")
      return { upstream: "shopify_admin", upstreamStatus: status };
    const databaseFailure = error as PostgreSqlFailure;
    if (
      (databaseFailure.name === "error" ||
        databaseFailure.name === "DatabaseError" ||
        databaseFailure.name === "PostgresError") &&
      typeof databaseFailure.code === "string" &&
      /^[0-9A-Z]{5}$/.test(databaseFailure.code)
    )
      return {
        upstream: "shopify_database",
        upstreamCode: databaseFailure.code,
      };
    if (error instanceof Error && SAFE_ERROR_NAMES.has(error.name))
      return { errorName: error.name };
  }
  return {};
}
