import type { LoaderFunctionArgs } from "react-router";

import { downloadPreviewDraftArtifact } from "../core/editor-preview.server";
import {
  readBoundedPdf,
  renderFirstPdfPagePng,
} from "../core/pdf-preview-image.server";
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
  try {
    const artifact = await downloadPreviewDraftArtifact(
      services.clientForLink(link),
      renderId,
    );
    if (!artifact.ok || !artifact.body) return unavailable(409);
    const image = await renderFirstPdfPagePng(await readBoundedPdf(artifact));
    return cors(
      new Response(Uint8Array.from(image).buffer, {
        headers: { "content-type": "image/png", ...PRIVATE_HEADERS },
      }),
    );
  } catch {
    return unavailable(422);
  }
}
