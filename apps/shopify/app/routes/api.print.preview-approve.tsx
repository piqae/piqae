import type { ActionFunctionArgs } from "react-router";

import { createProductionServices } from "../services.server";
import shopify from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;
export async function action({ request, params }: ActionFunctionArgs) {
  const { session, cors } = await shopify.authenticate.admin(request);
  const previewId = params.previewId ?? "";
  const body = (await request.json()) as Record<string, unknown>;
  const renderId = typeof body.renderId === "string" ? body.renderId : "";
  const printerId = typeof body.printerId === "string" ? body.printerId : "";
  const requestKey = request.headers.get("idempotency-key") ?? "";
  if (
    ![previewId, renderId, printerId].every((value) => ID.test(value)) ||
    !requestKey
  )
    return cors(
      Response.json({ error: "invalid approval request" }, { status: 400 }),
    );
  try {
    const result = await createProductionServices().printing.approvePreview({
      shop: session.shop,
      previewId,
      renderId,
      printerId,
      requestKey,
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
