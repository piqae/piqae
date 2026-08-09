import type { ActionFunctionArgs, LoaderFunctionArgs } from "react-router";
import shopify from "../shopify.server";
import { createProductionServices } from "../services.server";

async function execute(request: Request) {
  const { admin, session, cors } = await shopify.authenticate.admin(request);
  const url = new URL(request.url);
  const body = request.method === "POST" ? await request.formData() : null;
  const rawIds = String(
    body?.get("orderIds") ?? url.searchParams.get("orderIds") ?? "",
  );
  const printerId =
    request.method === "POST"
      ? String(body?.get("printerId") ?? "").trim() || undefined
      : undefined;
  const documents = String(
    body?.get("documents") ?? url.searchParams.get("documents") ?? "invoice",
  );
  if (!/^(invoice|packing_slip)(,(invoice|packing_slip))*$/.test(documents))
    return cors(
      Response.json({ error: "invalid document selection" }, { status: 400 }),
    );
  const result = await createProductionServices().printing.printOrders({
    admin,
    shop: session.shop,
    orderIds: rawIds
      .split(",")
      .map((v) => v.trim())
      .filter(Boolean),
    printerId,
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
