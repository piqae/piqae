import type { LoaderFunctionArgs } from "react-router";

import { downloadPreviewDraftArtifact } from "../core/editor-preview.server";
import { createProductionServices } from "../services.server";
import shopify from "../shopify.server";

const ID = /^[A-Za-z0-9_-]{1,128}$/;
const PRIVATE_HEADERS = {
  "cache-control": "private, no-store",
  "referrer-policy": "no-referrer",
  "x-content-type-options": "nosniff",
};

function unavailable(status = 404) {
  return new Response(null, { status, headers: PRIVATE_HEADERS });
}

export async function loader({ request, params }: LoaderFunctionArgs) {
  const { session, cors } = await shopify.authenticate.admin(request);
  const renderId = params.renderId ?? "";
  if (!ID.test(renderId)) return unavailable();
  const services = createProductionServices();
  if (!(await services.repository.ownsRender(session.shop, renderId)))
    return unavailable();
  const link = await services.repository.get(session.shop);
  if (!link) return unavailable();
  let upstream: Response;
  try {
    upstream = await downloadPreviewDraftArtifact(
      services.clientForLink(link),
      renderId,
    );
  } catch {
    return unavailable();
  }
  if (
    !upstream.ok ||
    !upstream.body ||
    upstream.headers.get("content-type")?.split(";", 1)[0] !== "application/pdf"
  )
    return unavailable(
      upstream.status === 404 || upstream.status === 410 ? 404 : 409,
    );
  return cors(
    new Response(upstream.body, {
      headers: {
        "content-type": "application/pdf",
        "content-disposition": `inline; filename="piqae-editor-preview-${renderId}.pdf"`,
        ...PRIVATE_HEADERS,
      },
    }),
  );
}
