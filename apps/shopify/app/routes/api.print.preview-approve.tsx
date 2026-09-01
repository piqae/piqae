import type { ActionFunctionArgs } from "react-router";
import { PiqaeError } from "@piqae/sdk";

import { createProductionServices } from "../services.server";
import { parseRenderCost } from "../core/printing.server";
import shopify from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;

export function approvalErrorMessage(error: unknown): string {
  if (error instanceof PiqaeError) {
    if (error.code === "node_render_not_ready")
      return "The selected Node is not ready for this document. Check the Node and try again.";
    if (error.code === "node_render_destination_unresolved")
      return "Piqae could not resolve the selected printer. Refresh the printer list and try again.";
    if (error.status >= 500 || error.code === "unexpected_response")
      return "Piqae could not reach the print service. The PDF is still available to download; try direct printing again in a moment.";
    return error.message;
  }
  if (error instanceof Error && error.message === "Bad Gateway")
    return "Piqae could not reach the print service. The PDF is still available to download; try direct printing again in a moment.";
  return error instanceof Error ? error.message : "Approval failed";
}

export function approvalErrorStatus(error: unknown): number {
  if (error instanceof PiqaeError)
    return error.status >= 500 || error.code === "unexpected_response"
      ? 502
      : 409;
  if (
    error instanceof TypeError ||
    (error instanceof Error && error.message === "Bad Gateway")
  )
    return 502;
  return 409;
}

function safeApprovalFailureMetadata(error: unknown) {
  if (error instanceof PiqaeError)
    return {
      upstreamCode: error.code,
      upstreamStatus: error.status,
      upstreamRequestId: error.requestId,
      retryable: error.retryable,
    };
  return error instanceof Error ? { errorName: error.name } : {};
}

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
    !ID.test(templateId) ||
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
      templateId,
      requestKey,
      renderCost,
    });
    return cors(Response.json(result, { status: 202 }));
  } catch (error) {
    const status = approvalErrorStatus(error);
    console.error(
      JSON.stringify({
        event: "shopify_admin_preview_approval_failed",
        status,
        ...safeApprovalFailureMetadata(error),
      }),
    );
    return cors(
      Response.json({ error: approvalErrorMessage(error) }, { status }),
    );
  }
}
