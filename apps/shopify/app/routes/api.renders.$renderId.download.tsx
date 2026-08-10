import type { LoaderFunctionArgs } from "react-router";
import shopify from "../shopify.server";
import { createProductionServices } from "../services.server";

export async function loader({ request, params }: LoaderFunctionArgs) {
  const { session, cors } = await shopify.authenticate.admin(request);
  const renderId = params.renderId ?? "";
  if (!/^[A-Za-z0-9_-]{1,128}$/.test(renderId))
    return Response.json({ error: "invalid render ID" }, { status: 400 });
  const services = createProductionServices();
  if (!(await services.repository.ownsRender(session.shop, renderId)))
    return Response.json({ error: "PDF not found" }, { status: 404 });
  const link = await services.repository.get(session.shop);
  if (!link)
    return Response.json(
      { error: "Piqae account is not connected" },
      { status: 409 },
    );
  const credential = services.vault.open(
    link.encryptedCredential,
    session.shop,
  );
  let upstream: Response | undefined;
  for (let attempt = 0; attempt < 20; attempt += 1) {
    upstream = await fetch(
      `${services.baseUrl}/v1/document-renders/${encodeURIComponent(renderId)}/artifact`,
      {
        headers: { authorization: `Bearer ${credential}` },
        signal: AbortSignal.timeout(10_000),
      },
    );
    if (upstream.ok || upstream.status !== 409) break;
    await new Promise((resolve) =>
      setTimeout(resolve, Math.min(2_000, 100 * 2 ** attempt)),
    );
  }
  if (!upstream?.ok || !upstream.body)
    return Response.json(
      {
        error:
          upstream?.status === 409
            ? "PDF is still rendering"
            : "PDF unavailable",
      },
      { status: upstream?.status === 404 ? 404 : 409 },
    );
  return cors(
    new Response(upstream.body, {
      headers: {
        "content-type": "application/pdf",
        "content-disposition": `inline; filename="shopify-document-${renderId}.pdf"`,
        "cache-control": "private, no-store",
      },
    }),
  );
}
