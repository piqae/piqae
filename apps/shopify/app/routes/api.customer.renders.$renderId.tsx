import type { LoaderFunctionArgs } from "react-router";
import shopify from "../shopify.server";
import { createProductionServices } from "../services.server";
import { normalizeOrderGid } from "../core/orders.server";

export async function loader({ request, params }: LoaderFunctionArgs) {
  const { sessionToken, cors } =
    await shopify.authenticate.public.customerAccount(request);
  const shop = new URL(sessionToken.dest).hostname;
  const customerGid = String(sessionToken.sub ?? "");
  if (!/^gid:\/\/shopify\/Customer\/[1-9][0-9]*$/.test(customerGid))
    return cors(
      Response.json({ error: "customer identity required" }, { status: 403 }),
    );
  const renderId = params.renderId ?? "";
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(renderId))
    return cors(Response.json({ error: "invalid render" }, { status: 400 }));
  let orderGid: string;
  try {
    orderGid = normalizeOrderGid(
      new URL(request.url).searchParams.get("orderId") ?? "",
    );
  } catch {
    return cors(Response.json({ error: "invalid order" }, { status: 400 }));
  }
  const services = createProductionServices();
  if (
    !(await services.repository.ownsCustomerRender(
      shop,
      renderId,
      orderGid,
      customerGid,
    ))
  )
    return cors(Response.json({ error: "PDF not found" }, { status: 404 }));
  const link = await services.repository.get(shop);
  if (!link)
    return cors(Response.json({ error: "PDF not found" }, { status: 404 }));
  const upstream = await services
    .clientForLink(link)
    .printPackets.renders.download(renderId);
  if (!upstream.ok || !upstream.body)
    return cors(
      Response.json(
        { error: "PDF unavailable" },
        { status: upstream.status === 404 ? 404 : 409 },
      ),
    );
  return cors(
    new Response(upstream.body, {
      headers: {
        "content-type": "application/pdf",
        "content-disposition": `inline; filename="order-${encodeURIComponent(orderGid.split("/").pop()!)}.pdf"`,
        "cache-control": "private, no-store",
      },
    }),
  );
}
