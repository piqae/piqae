import type { LoaderFunctionArgs } from "react-router";

import { adminExtensionCors } from "../../server/admin-extension-cors.mjs";
import { createProductionServices } from "../services.server";

export async function loader({ request }: LoaderFunctionArgs) {
  const services = createProductionServices();
  const url = new URL(request.url);
  const download = url.searchParams.get("download") === "1";
  let grant;
  try {
    grant = services.downloadTokens.openPreview(
      url.searchParams.get("token") ?? "",
    );
  } catch {
    return adminExtensionCors(new Response(null, { status: 404 }), request);
  }
  if (!(await services.repository.ownsRender(grant.shop, grant.renderId)))
    return adminExtensionCors(new Response(null, { status: 404 }), request);
  const link = await services.repository.get(grant.shop);
  if (!link)
    return adminExtensionCors(new Response(null, { status: 404 }), request);
  const client = services.clientForLink(link);
  const preview = await client.printPackets.previews.retrieve(grant.previewId);
  if (preview.render_id !== grant.renderId)
    return adminExtensionCors(new Response(null, { status: 404 }), request);
  const artifact = await client.printPackets.previews.download(grant.previewId);
  if (!artifact.ok || !artifact.body)
    return adminExtensionCors(
      new Response(null, { status: artifact.status }),
      request,
    );
  return adminExtensionCors(
    new Response(artifact.body, {
      headers: {
        "content-type": "application/pdf",
        "content-disposition": `${download ? "attachment" : "inline"}; filename="piqae-preview-${grant.previewId}.pdf"`,
        "cache-control": "private, no-store",
        "cross-origin-resource-policy": "cross-origin",
        "referrer-policy": "no-referrer",
        "x-content-type-options": "nosniff",
      },
    }),
    request,
  );
}
