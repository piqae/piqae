import type { ActionFunctionArgs } from "react-router";

import { createProductionServices } from "../services.server";
import { DocumentRenderFailedError } from "../core/document-render-errors";
import { ShopifyOrderUnavailableError } from "../core/shopify-order-errors";
import { safeFailureMetadata } from "../core/safe-failure-metadata.server";
import shopify, { migrateLegacyOfflineSession } from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;

export type AdminPreviewFailure = {
  code:
    | "document_publication"
    | "order_access_window"
    | "order_data"
    | "render_service"
    | "account_connection"
    | "preview_failed";
  message: string;
};

type ShopifyHttpFailure = {
  response?: {
    code?: unknown;
    body?: unknown;
  };
};

export class ShopifySessionRecoveryError extends Error {
  override readonly name = "ShopifySessionRecoveryError";

  constructor(cause: unknown) {
    super("Shopify access-token recovery failed", { cause });
  }
}

/**
 * Shopify's client normally invalidates an offline session on HTTP 401. Some
 * invalid or revoked offline tokens are returned as HTTP 403 instead, so the
 * client leaves that session in storage forever. Match only Shopify's
 * credential-shaped 403 response; an ordinary permission failure must not
 * trigger a token exchange loop.
 */
export function isShopifySessionCredentialFailure(error: unknown): boolean {
  if (!error || typeof error !== "object") return false;
  const response = (error as ShopifyHttpFailure).response;
  if (!response || typeof response.code !== "number") return false;
  if (response.code === 401) return true;
  if (
    response.code !== 403 ||
    !response.body ||
    typeof response.body !== "object"
  )
    return false;
  const errors = (response.body as Record<string, unknown>).errors;
  return (
    typeof errors === "string" &&
    /(?:access token|api key|credential)/i.test(errors)
  );
}

export function isLegacyNonExpiringTokenFailure(error: unknown): boolean {
  if (!error || typeof error !== "object") return false;
  const response = (error as ShopifyHttpFailure).response;
  if (
    response?.code !== 403 ||
    !response.body ||
    typeof response.body !== "object"
  )
    return false;
  const errors = (response.body as Record<string, unknown>).errors;
  return (
    typeof errors === "string" &&
    /non-expiring access tokens are no longer accepted/i.test(errors)
  );
}

export async function withShopifySessionRecovery<T>(
  operation: () => Promise<T>,
  recover: (error: unknown) => Promise<T>,
): Promise<T> {
  try {
    return await operation();
  } catch (error) {
    if (!isShopifySessionCredentialFailure(error)) throw error;
    return recover(error);
  }
}

/**
 * Keep production logs useful without copying order ids, shop domains, document
 * contents, or upstream response bodies into them. The original error is still
 * converted into a merchant-facing instruction when it is safe and specific.
 */
export function classifyAdminPreviewFailure(
  error: unknown,
  resourceType: "orders" | "products" = "orders",
): AdminPreviewFailure {
  if (
    isShopifySessionCredentialFailure(error) ||
    error instanceof ShopifySessionRecoveryError
  )
    return {
      code: "account_connection",
      message:
        "Shopify access could not be refreshed. Open Piqae in Shopify Admin once, then retry this print action.",
    };
  if (
    error instanceof DocumentRenderFailedError &&
    error.failureCode === "document_data_missing"
  )
    return {
      code: "order_data",
      message:
        "This document requires data that was not available for the selection. Add a fallback or condition to the missing field in the template, then try again.",
    };
  if (
    error instanceof DocumentRenderFailedError &&
    (error.failureCode === "renderer_version_unsupported" ||
      error.failureCode === "renderer_feature_unsupported")
  )
    return {
      code: "render_service",
      message:
        "This document uses a renderer capability that is not active across Piqae yet. Republish the document after the update completes, then try again.",
    };
  if (
    resourceType === "orders" &&
    error instanceof ShopifyOrderUnavailableError &&
    error.reason === "standard_history_only"
  )
    return {
      code: "order_access_window",
      message:
        "Shopify did not make one or more selected orders available to Piqae. This installation can access the last 60 days; older orders require Shopify's all-orders permission.",
    };
  const message = error instanceof Error ? error.message : "";
  if (
    /published|pinned piqae revision|template revision|document.*unavailable/i.test(
      message,
    )
  )
    return {
      code: "document_publication",
      message:
        "This document publication is no longer available. Open the document, publish it again, then retry the preview.",
    };
  if (
    resourceType === "products" &&
    /product|variant|shopify.*data|graphql/i.test(message)
  )
    return {
      code: "order_data",
      message:
        "Piqae could not load every selected Shopify product or variant. Refresh the products and try again.",
    };
  if (/order|shopify.*data|graphql/i.test(message))
    return {
      code: "order_data",
      message:
        "Piqae could not load the selected Shopify order data. Refresh the orders and try again.",
    };
  if (/render|preview|artifact/i.test(message))
    return {
      code: "render_service",
      message: "Piqae could not generate this preview. Try again in a moment.",
    };
  if (/connect|credential|account|environment/i.test(message))
    return {
      code: "account_connection",
      message:
        "The Piqae connection needs attention. Reconnect the Node from Piqae settings, then retry.",
    };
  return {
    code: "preview_failed",
    message: "Piqae could not generate this preview. Try again in a moment.",
  };
}

export async function action({ request }: ActionFunctionArgs) {
  const reauthorizationRequest = request.clone();
  const { admin, session, cors } = await shopify.authenticate.admin(request);
  const body = (await request.json()) as Record<string, unknown>;
  const orderIds = Array.isArray(body.orderIds)
    ? body.orderIds.filter(
        (value): value is string => typeof value === "string",
      )
    : [];
  const templateId = typeof body.templateId === "string" ? body.templateId : "";
  const requestKey = request.headers.get("idempotency-key") ?? "";
  if (!requestKey || !templateId || !ID.test(templateId))
    return cors(
      Response.json({ error: "invalid preview request" }, { status: 400 }),
    );
  const preview = (previewAdmin: typeof admin, shop: string) =>
    createProductionServices().printing.previewOrders({
      admin: previewAdmin,
      shop,
      orderIds,
      templateId,
      requestKey,
    });
  try {
    const result = await withShopifySessionRecovery(
      () => preview(admin, session.shop),
      async (credentialFailure) => {
        let refreshed;
        try {
          if (isLegacyNonExpiringTokenFailure(credentialFailure)) {
            await migrateLegacyOfflineSession(session);
            refreshed = await shopify.unauthenticated.admin(session.shop);
          } else {
            // The retry request still carries Shopify's short-lived admin
            // session token. Removing only the invalid offline session makes
            // the official strategy exchange it for a fresh offline token.
            await shopify.sessionStorage.deleteSession(session.id);
            refreshed = await shopify.authenticate.admin(
              reauthorizationRequest,
            );
          }
        } catch (error) {
          throw new ShopifySessionRecoveryError(error);
        }
        return preview(refreshed.admin, refreshed.session.shop);
      },
    );
    return cors(Response.json(result, { status: 201 }));
  } catch (error) {
    const failure = classifyAdminPreviewFailure(error);
    console.error(
      JSON.stringify({
        event: "shopify_admin_preview_failed",
        code: failure.code,
        ...safeFailureMetadata(error),
      }),
    );
    return cors(
      Response.json(
        { error: failure.message, code: failure.code },
        { status: 422 },
      ),
    );
  }
}
