import type { LoaderFunctionArgs } from "react-router";

import { PiqaeClient } from "@piqae/sdk";
import { createProductionServices } from "../services.server";
import shopify from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;
export async function loader({ request, params }: LoaderFunctionArgs) {
  const { session, cors } = await shopify.authenticate.admin(request);
  const previewId = params.previewId ?? "";
  const renderId = new URL(request.url).searchParams.get("renderId") ?? "";
  if (!ID.test(previewId) || !ID.test(renderId))
    return new Response(null, { status: 404 });
  const services = createProductionServices();
  if (!(await services.repository.ownsRender(session.shop, renderId)))
    return new Response(null, { status: 404 });
  const link = await services.repository.get(session.shop);
  if (!link) return new Response(null, { status: 404 });
  const client = new PiqaeClient({
    baseUrl: services.baseUrl,
    accessToken: () =>
      services.vault.open(link.encryptedCredential, session.shop),
  });
  const preview = await client.documents.previews.retrieve(previewId);
  if (preview.render_id !== renderId)
    return new Response(null, { status: 404 });
  const artifact = await client.documents.previews.download(previewId);
  if (!artifact.ok || !artifact.body)
    return new Response(null, { status: artifact.status });
  return cors(
    new Response(artifact.body, {
      headers: {
        "content-type": "application/pdf",
        "content-disposition": `inline; filename="piqae-preview-${previewId}.pdf"`,
        "cache-control": "private, no-store",
      },
    }),
  );
}
