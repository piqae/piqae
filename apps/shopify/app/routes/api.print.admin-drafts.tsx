import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import shopify from "../shopify.server";
import { createProductionServices } from "../services.server";

async function execute(request: Request) {
  const { admin, session, cors } = await shopify.authenticate.admin(request);
  const url = new URL(request.url);
  const body = request.method === "POST" ? await request.formData() : null;
  const ids = String(
    body?.get("draftOrderIds") ?? url.searchParams.get("draftOrderIds") ?? "",
  )
    .split(",")
    .map((id) => id.trim())
    .filter(Boolean);
  const printerId =
    request.method === "POST"
      ? String(body?.get("printerId") ?? "").trim() || undefined
      : undefined;
  const result = await createProductionServices().printing.printOrders({
    admin,
    shop: session.shop,
    orderIds: ids,
    printerId,
    resourceType: "draft_orders",
    requestKey: request.headers.get("idempotency-key") ?? undefined,
  });
  if (request.method === "GET" && result.mode === "download")
    return cors(Response.redirect(result.downloadUrl, 303));
  return cors(Response.json(result, { status: 202 }));
}
export function loader({ request }: LoaderFunctionArgs) {
  return execute(request);
}
export function action({ request }: ActionFunctionArgs) {
  return execute(request);
}
