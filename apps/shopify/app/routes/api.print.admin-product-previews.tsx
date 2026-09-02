import type { ActionFunctionArgs } from "react-router";

import { createProductionServices } from "../services.server";
import {
  ShopifySessionRecoveryError,
  classifyAdminPreviewFailure,
  isLegacyNonExpiringTokenFailure,
  safeFailureMetadata,
  withShopifySessionRecovery,
} from "./api.print.admin-previews";
import shopify, { migrateLegacyOfflineSession } from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;

export async function action({ request }: ActionFunctionArgs) {
  const reauthorizationRequest = request.clone();
  const { admin, session, cors } = await shopify.authenticate.admin(request);
  const body = (await request.json()) as Record<string, unknown>;
  const productIds = Array.isArray(body.productIds)
    ? body.productIds.filter(
        (value): value is string => typeof value === "string",
      )
    : [];
  const templateId = typeof body.templateId === "string" ? body.templateId : "";
  const requestKey = request.headers.get("idempotency-key") ?? "";
  if (
    !requestKey ||
    !ID.test(templateId) ||
    productIds.length < 1 ||
    productIds.length > 100
  )
    return cors(
      Response.json(
        { error: "invalid product preview request" },
        { status: 400 },
      ),
    );
  const preview = (previewAdmin: typeof admin, shop: string) =>
    createProductionServices().printing.previewProducts({
      admin: previewAdmin,
      shop,
      productIds,
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
    const failure = classifyAdminPreviewFailure(error, "products");
    console.error(
      JSON.stringify({
        event: "shopify_admin_product_preview_failed",
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
