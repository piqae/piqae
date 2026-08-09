import type { LoaderFunctionArgs } from "react-router";
import { createProductionServices } from "../services.server";

export async function loader({ request }: LoaderFunctionArgs) {
  const services = createProductionServices();
  let grant;
  try {
    grant = services.downloadTokens.open(
      new URL(request.url).searchParams.get("token") ?? "",
    );
  } catch {
    return Response.json(
      { error: "download link expired or invalid" },
      { status: 403 },
    );
  }
  if (
    !(await services.repository.ownsCustomerRender(
      grant.shop,
      grant.renderId,
      grant.orderGid,
      grant.customerGid,
    ))
  )
    return Response.json({ error: "PDF not found" }, { status: 404 });
  const link = await services.repository.get(grant.shop);
  if (!link) return Response.json({ error: "PDF not found" }, { status: 404 });
  const credential = services.vault.open(link.encryptedCredential, grant.shop);
  const upstream = await fetch(
    `${services.baseUrl}/v1/document-renders/${encodeURIComponent(grant.renderId)}/artifact`,
    { headers: { authorization: `Bearer ${credential}` }, signal: AbortSignal.timeout(10_000) },
  );
  if (!upstream.ok || !upstream.body)
    return Response.json(
      { error: "PDF unavailable" },
      { status: upstream.status === 404 ? 404 : 409 },
    );
  return new Response(upstream.body, {
    headers: {
      "content-type": "application/pdf",
      "content-disposition": `inline; filename="order-${grant.orderGid.split("/").pop()}.pdf"`,
      "cache-control": "private, no-store",
      "referrer-policy": "no-referrer",
    },
  });
}
