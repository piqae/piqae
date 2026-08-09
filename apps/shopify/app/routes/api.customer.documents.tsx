import type { LoaderFunctionArgs } from "react-router";
import shopify from "../shopify.server";
import { createProductionServices } from "../services.server";
import { fetchOrders, normalizeOrderGid } from "../core/orders.server";

export async function loader({ request }: LoaderFunctionArgs) {
  const { sessionToken, cors } =
    await shopify.authenticate.public.customerAccount(request);
  const shop = new URL(sessionToken.dest).hostname;
  const customerGid = String(sessionToken.sub ?? "");
  if (!/^gid:\/\/shopify\/Customer\/[1-9][0-9]*$/.test(customerGid))
    return cors(
      Response.json({ error: "customer identity required" }, { status: 403 }),
    );
  let orderGid: string;
  try {
    orderGid = normalizeOrderGid(
      new URL(request.url).searchParams.get("orderId") ?? "",
    );
  } catch {
    return cors(Response.json({ error: "invalid order" }, { status: 400 }));
  }
  const services = createProductionServices();
  let renderId = await services.repository.latestCustomerRender(
    shop,
    orderGid,
    customerGid,
  );
  if (!renderId) {
    const { admin } = await shopify.unauthenticated.admin(shop);
    const [order] = await fetchOrders(admin, [orderGid]);
    if (!order || order.customer?.id !== customerGid)
      return cors(Response.json({ error: "order not found" }, { status: 404 }));
    const result = await services.printing.printOrders({
      admin,
      shop,
      orderIds: [orderGid],
      requestKey: `customer-${customerGid}-${orderGid}`,
    });
    renderId = result.renderId;
  }
  const token = services.downloadTokens.issue({
    shop,
    renderId,
    orderGid,
    customerGid,
  });
  const url = `${process.env.SHOPIFY_APP_URL}/api/public/documents/download?token=${encodeURIComponent(token)}`;
  return cors(
    Response.json(
      { renderId, downloadUrl: url },
      { headers: { "cache-control": "private, no-store" } },
    ),
  );
}
