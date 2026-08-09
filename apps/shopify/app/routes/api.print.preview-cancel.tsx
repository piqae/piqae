import type { ActionFunctionArgs } from "react-router";

import { createProductionServices } from "../services.server";
import shopify from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;
export async function action({ request, params }: ActionFunctionArgs) {
  const { session, cors } = await shopify.authenticate.admin(request);
  const previewId = params.previewId ?? "";
  const body = (await request.json()) as Record<string, unknown>;
  const renderId = typeof body.renderId === "string" ? body.renderId : "";
  const requestKey = request.headers.get("idempotency-key") ?? "";
  if (!ID.test(previewId) || !ID.test(renderId) || !requestKey)
    return cors(
      Response.json({ error: "invalid cancellation request" }, { status: 400 }),
    );
  try {
    const preview = await createProductionServices().printing.cancelPreview({
      shop: session.shop,
      previewId,
      renderId,
      requestKey,
    });
    return cors(Response.json({ state: preview.state }));
  } catch (error) {
    return cors(
      Response.json(
        {
          error: error instanceof Error ? error.message : "Cancellation failed",
        },
        { status: 409 },
      ),
    );
  }
}
