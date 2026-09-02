import type { ActionFunctionArgs } from "react-router";

import { createProductionServices } from "../services.server";
import shopify from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;

export async function action({ request }: ActionFunctionArgs) {
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
  try {
    const result = await createProductionServices().printing.previewProducts({
      admin,
      shop: session.shop,
      productIds,
      templateId,
      requestKey,
    });
    return cors(Response.json(result, { status: 201 }));
  } catch {
    return cors(
      Response.json(
        { error: "Piqae could not generate the selected product labels" },
        { status: 422 },
      ),
    );
  }
}
