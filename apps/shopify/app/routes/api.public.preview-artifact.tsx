import type { LoaderFunctionArgs } from "react-router";

import { PiqaeClient } from "@piqae/sdk";
import { createProductionServices } from "../services.server";

export async function loader({ request }: LoaderFunctionArgs) {
  const services = createProductionServices();
  let grant;
  try {
    grant = services.downloadTokens.openPreview(
      new URL(request.url).searchParams.get("token") ?? "",
    );
  } catch {
    return new Response(null, { status: 404 });
  }
  if (!(await services.repository.ownsRender(grant.shop, grant.renderId)))
    return new Response(null, { status: 404 });
  const link = await services.repository.get(grant.shop);
  if (!link) return new Response(null, { status: 404 });
  const client = new PiqaeClient({
    baseUrl: services.baseUrl,
    accessToken: () =>
      services.vault.open(link.encryptedCredential, grant.shop),
  });
  const preview = await client.documents.previews.retrieve(grant.previewId);
  if (preview.render_id !== grant.renderId)
    return new Response(null, { status: 404 });
  const artifact = await client.documents.previews.download(grant.previewId);
  if (!artifact.ok || !artifact.body)
    return new Response(null, { status: artifact.status });
  return new Response(artifact.body, {
    headers: {
      "content-type": "application/pdf",
      "content-disposition": `inline; filename="piqae-preview-${grant.previewId}.pdf"`,
      "cache-control": "private, no-store",
      "referrer-policy": "no-referrer",
      "x-content-type-options": "nosniff",
    },
  });
}
