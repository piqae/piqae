import { describe, expect, it } from "vitest";

import {
  classifyAdminPreviewFailure,
  isLegacyNonExpiringTokenFailure,
  isShopifySessionCredentialFailure,
  ShopifySessionRecoveryError,
  withShopifySessionRecovery,
} from "../app/routes/api.print.admin-previews";
import {
  fetchOrders,
  ShopifyAdminGraphqlError,
} from "../app/core/orders.server";
import { ShopifyOrderUnavailableError } from "../app/core/shopify-order-errors";
import { DocumentRenderFailedError } from "../app/core/document-render-errors";
import { safeFailureMetadata } from "../app/core/safe-failure-metadata.server";

describe("admin preview failure classification", () => {
  it("turns stale publications into a useful republish instruction", () => {
    expect(
      classifyAdminPreviewFailure(
        new Error("The published packing slip has no pinned Piqae revision"),
      ),
    ).toEqual({
      code: "document_publication",
      message:
        "This document publication is no longer available. Open the document, publish it again, then retry the preview.",
    });
  });

  it("explains Shopify's order-history window without claiming the missing order is old", () => {
    expect(
      classifyAdminPreviewFailure(
        new ShopifyOrderUnavailableError("standard_history_only"),
      ),
    ).toEqual({
      code: "order_access_window",
      message:
        "Shopify did not make one or more selected orders available to Piqae. This installation can access the last 60 days; older orders require Shopify's all-orders permission.",
    });
  });

  it("does not expose unknown upstream error text", () => {
    expect(
      classifyAdminPreviewFailure(new Error("sensitive upstream body")),
    ).toEqual({
      code: "preview_failed",
      message: "Piqae could not generate this preview. Try again in a moment.",
    });
  });

  it("logs only the renderer failure code for a terminal render", () => {
    const failure = new DocumentRenderFailedError(
      "document_render_failed",
      "document",
    );

    expect(classifyAdminPreviewFailure(failure)).toEqual({
      code: "render_service",
      message: "Piqae could not generate this preview. Try again in a moment.",
    });
    expect(safeFailureMetadata(failure)).toEqual({
      renderFailureCode: "document_render_failed",
    });
  });

  it("turns renderer data and version failures into actionable bounded guidance", () => {
    expect(
      classifyAdminPreviewFailure(
        new DocumentRenderFailedError("document_data_missing"),
      ),
    ).toEqual({
      code: "order_data",
      message:
        "This document requires data that was not available for the selection. Add a fallback or condition to the missing field in the template, then try again.",
    });
    expect(
      classifyAdminPreviewFailure(
        new DocumentRenderFailedError("renderer_version_unsupported"),
      ),
    ).toEqual({
      code: "render_service",
      message:
        "This document uses a renderer capability that is not active across Piqae yet. Republish the document after the update completes, then try again.",
    });
  });

  it("reports a bounded Shopify GraphQL failure without logging its body", () => {
    const failure = new ShopifyAdminGraphqlError();
    expect(classifyAdminPreviewFailure(failure)).toEqual({
      code: "order_data",
      message:
        "Piqae could not load the selected Shopify order data. Refresh the orders and try again.",
    });
    expect(safeFailureMetadata(failure)).toEqual({
      upstream: "shopify_admin",
      failureKind: "graphql_query",
    });
  });

  it("redacts unknown renderer codes and untrusted object names", () => {
    expect(
      safeFailureMetadata(
        new DocumentRenderFailedError("buyer@example.test", "document"),
      ),
    ).toEqual({ renderFailureCode: "unknown_render_failure" });
    expect(
      safeFailureMetadata({ name: "buyer@example.test", response: {} }),
    ).toEqual({});
  });

  it("classifies missing product data without referring to orders", () => {
    expect(
      classifyAdminPreviewFailure(
        new Error("Shopify ProductVariant node was unavailable"),
        "products",
      ),
    ).toEqual({
      code: "order_data",
      message:
        "Piqae could not load every selected Shopify product or variant. Refresh the products and try again.",
    });
  });

  it("surfaces a failed Shopify token migration as an account issue", () => {
    expect(
      classifyAdminPreviewFailure(
        new ShopifySessionRecoveryError(new Error("sensitive OAuth body")),
      ),
    ).toEqual({
      code: "account_connection",
      message:
        "Shopify access could not be refreshed. Open Piqae in Shopify Admin once, then retry this print action.",
    });
  });

  it("recognizes revoked Shopify offline credentials returned as HTTP 403", () => {
    const revoked = {
      response: {
        code: 403,
        body: { errors: "The access token for this shop is invalid" },
      },
    };
    expect(isShopifySessionCredentialFailure(revoked)).toBe(true);
    expect(classifyAdminPreviewFailure(revoked)).toEqual({
      code: "account_connection",
      message:
        "Shopify access could not be refreshed. Open Piqae in Shopify Admin once, then retry this print action.",
    });
    expect(
      isShopifySessionCredentialFailure({
        response: {
          code: 403,
          body: { errors: "This operation requires another permission" },
        },
      }),
    ).toBe(false);
  });

  it("distinguishes Shopify's one-time legacy offline-token migration", () => {
    const legacy = {
      response: {
        code: 403,
        body: {
          errors:
            "[API] Non-expiring access tokens are no longer accepted for the Admin API. Start using expiring offline tokens.",
        },
      },
    };
    expect(isShopifySessionCredentialFailure(legacy)).toBe(true);
    expect(isLegacyNonExpiringTokenFailure(legacy)).toBe(true);
    expect(
      isLegacyNonExpiringTokenFailure({
        response: {
          code: 403,
          body: { errors: "The access token for this shop is invalid" },
        },
      }),
    ).toBe(false);
  });

  it("recognizes a credential failure returned by the order loader", async () => {
    const admin = {
      graphql: async () =>
        Response.json(
          { errors: "The access token for this shop is invalid" },
          { status: 403 },
        ),
    };

    const failure = await fetchOrders(admin, ["42"]).catch(
      (error: unknown) => error,
    );
    expect(isShopifySessionCredentialFailure(failure)).toBe(true);
  });

  it("recovers a revoked session once without retrying unrelated failures", async () => {
    const revoked = {
      response: {
        code: 403,
        body: { errors: "The access token for this shop is invalid" },
      },
    };
    let recoveryCalls = 0;
    await expect(
      withShopifySessionRecovery(
        async () => Promise.reject(revoked),
        async (error) => {
          expect(error).toBe(revoked);
          recoveryCalls += 1;
          return "preview-ready";
        },
      ),
    ).resolves.toBe("preview-ready");
    expect(recoveryCalls).toBe(1);

    await expect(
      withShopifySessionRecovery(
        async () => Promise.reject(new Error("render unavailable")),
        async () => "must-not-run",
      ),
    ).rejects.toThrow("render unavailable");
  });
});
