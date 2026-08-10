import type { LoaderFunctionArgs } from "react-router";

export function loader(_args: LoaderFunctionArgs): Response {
  return Response.json(
    { status: "ok", service: "piqae-shopify", version: "0.1.0" },
    { headers: { "cache-control": "no-store" } },
  );
}
