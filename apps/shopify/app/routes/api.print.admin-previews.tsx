import type { ActionFunctionArgs } from "react-router";

import { createProductionServices } from "../services.server";
import { adminExtensionPreflight } from "../core/admin-extension-cors.server";
import shopify from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;

export async function action({ request }: ActionFunctionArgs) {
  const preflight = adminExtensionPreflight(request);
  if (preflight) return preflight;
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
    return cors(
      Response.json(
        { error: error instanceof Error ? error.message : "Preview failed" },
        { status: 422 },
      ),
    );
  }
}
