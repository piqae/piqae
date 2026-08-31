import type { ActionFunctionArgs } from "react-router";

import { createProductionServices } from "../services.server";
import { parseRenderCost } from "../core/printing.server";
import shopify from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;
export async function action({ request, params }: ActionFunctionArgs) {
  const { session, cors } = await shopify.authenticate.admin(request);
  const previewId = params.previewId ?? "";
  const body = (await request.json()) as Record<string, unknown>;
  const renderId = typeof body.renderId === "string" ? body.renderId : "";
  const printerId = typeof body.printerId === "string" ? body.printerId : "";
  const targetId = typeof body.targetId === "string" ? body.targetId : "";
  const templateId = typeof body.templateId === "string" ? body.templateId : "";
  const specificationRevision =
    typeof body.specificationRevision === "string"
      ? body.specificationRevision
      : "";
  const requestKey = request.headers.get("idempotency-key") ?? "";
  if (
    ![previewId, renderId].every((value) => ID.test(value)) ||
    !ID.test(targetId || printerId) ||
    Boolean(targetId) === Boolean(printerId) ||
    Boolean(targetId) !== Boolean(specificationRevision) ||
    (specificationRevision && !ID.test(specificationRevision)) ||
    (targetId && !ID.test(templateId)) ||
    !requestKey
  )
    return cors(
      Response.json({ error: "invalid approval request" }, { status: 400 }),
    );
  try {
    const renderCost = parseRenderCost(body.renderCost);
    const result = await createProductionServices().printing.approvePreview({
      shop: session.shop,
      previewId,
      renderId,
      printerId: printerId || undefined,
      targetId: targetId || undefined,
      targetSpecificationRevision: specificationRevision || undefined,
      templateId: templateId || undefined,
      requestKey,
      renderCost,
    });
    return cors(Response.json(result, { status: 202 }));
  } catch (error) {
    return cors(
      Response.json(
        { error: error instanceof Error ? error.message : "Approval failed" },
        { status: 409 },
      ),
    );
  }
}
