import type { LoaderFunctionArgs } from "react-router";
import shopify from "../shopify.server";
import { createProductionServices } from "../services.server";
import { fetchOrders } from "../core/orders.server";

export async function loader({ request }: LoaderFunctionArgs) {
  const { sessionToken, cors } = await shopify.authenticate.pos(request);
  const shop = new URL(sessionToken.dest).hostname;
  const { admin } = await shopify.unauthenticated.admin(shop);
  const url = new URL(request.url);
  if ((url.searchParams.get("document") ?? "receipt") !== "receipt")
    return cors(Response.json({ error: "invalid document" }, { status: 400 }));
  const orderId = url.searchParams.get("orderId") ?? "";
  const format = url.searchParams.get("format") ?? "pdf";
  if (format === "html") {
    const [order] = await fetchOrders(admin, [orderId]);
    if (!order)
      return cors(Response.json({ error: "order not found" }, { status: 404 }));
    return cors(
      new Response(receiptHtml(order), {
        headers: {
          "content-type": "text/html; charset=utf-8",
          "cache-control": "private, no-store",
          "content-security-policy":
            "default-src 'none'; style-src 'unsafe-inline'; img-src data:",
        },
      }),
    );
  }
  if (format !== "pdf")
    return cors(Response.json({ error: "invalid format" }, { status: 400 }));
  const result = await createProductionServices().printing.printOrders({
    admin,
    shop,
    orderIds: [orderId],
    requestKey: request.headers.get("idempotency-key") ?? undefined,
  });
  if (result.mode === "download")
    return cors(Response.redirect(result.downloadUrl, 303));
  return cors(Response.json(result, { status: 202 }));
}

function escape(value: unknown): string {
  return String(value ?? "").replace(
    /[&<>"']/g,
    (character) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        character
      ]!,
  );
}
function receiptHtml(
  order: Awaited<ReturnType<typeof fetchOrders>>[number],
): string {
  const rows = order.lineItems
    .map(
      (line) =>
        `<tr><td>${escape(line.quantity)} × ${escape(line.title)}</td><td>${escape(line.total)}</td></tr>`,
    )
    .join("");
  return `<!doctype html><html><head><meta charset="utf-8"><title>${escape(order.name)}</title><style>@page{margin:4mm}body{font:12px system-ui,sans-serif;margin:0;color:#000}h1{font-size:18px}table{width:100%;border-collapse:collapse}td{padding:3px 0}td:last-child{text-align:right}.total{font-weight:700;border-top:1px solid}</style></head><body><h1>${escape(order.name)}</h1><p>${escape(new Date(order.createdAt).toLocaleString("en-US", { timeZone: "UTC" }))} UTC</p><table>${rows}<tr><td>Tax</td><td>${escape(order.tax)} ${escape(order.currency)}</td></tr><tr class="total"><td>Total</td><td>${escape(order.total)} ${escape(order.currency)}</td></tr></table></body></html>`;
}
