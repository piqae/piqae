import type { ActionFunctionArgs } from "react-router";

import { createProductionServices } from "../services.server";
import shopify from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;

export type AdminPreviewFailure = {
  code:
    | "document_publication"
    | "order_data"
    | "render_service"
    | "account_connection"
    | "preview_failed";
  message: string;
};

/**
 * Keep production logs useful without copying order ids, shop domains, document
 * contents, or upstream response bodies into them. The original error is still
 * converted into a merchant-facing instruction when it is safe and specific.
 */
export function classifyAdminPreviewFailure(
  error: unknown,
): AdminPreviewFailure {
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
  try {
    const result = await createProductionServices().printing.previewOrders({
      admin,
      shop: session.shop,
      orderIds,
      templateId,
      requestKey,
    });
    return cors(Response.json(result, { status: 201 }));
  } catch (error) {
    const failure = classifyAdminPreviewFailure(error);
    console.error(
      JSON.stringify({
        event: "shopify_admin_preview_failed",
        code: failure.code,
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
