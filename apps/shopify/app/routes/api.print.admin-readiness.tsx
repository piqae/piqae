import type { ActionFunctionArgs } from "react-router";

import { createProductionServices } from "../services.server";
import { parseRenderCost } from "../core/printing.server";
import shopify from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;

export async function action({ request }: ActionFunctionArgs) {
  const { session, cors } = await shopify.authenticate.admin(request);
  const body = (await request.json()) as Record<string, unknown>;
  const renderId = typeof body.renderId === "string" ? body.renderId : "";
  const printerId = typeof body.printerId === "string" ? body.printerId : "";
  if (!ID.test(renderId) || !ID.test(printerId))
    return cors(
      Response.json({ error: "invalid readiness request" }, { status: 400 }),
    );
  try {
    const renderCost = parseRenderCost(body.renderCost);
    const readiness = await createProductionServices().printing.renderReadiness(
      {
        shop: session.shop,
        renderId,
        printerId,
        renderCost,
      },
    );
    return cors(Response.json(readiness));
  } catch (error) {
    return cors(
      Response.json(
        {
          error:
            error instanceof Error
              ? error.message
              : "Node rendering readiness could not be checked",
        },
        { status: 422 },
      ),
    );
  }
}
