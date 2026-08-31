const ADMIN_EXTENSION_ORIGIN = "https://extensions.shopifycdn.com";
const ALLOWED_REQUEST_HEADERS = new Set([
  "authorization",
  "content-type",
  "idempotency-key",
]);

/**
 * React Router sends OPTIONS requests for action-only resource routes into the
 * action. Shopify authentication correctly rejects that unauthenticated
 * preflight, so answer it before authentication while keeping the real POST
 * protected by the signed Admin extension token.
 */
export function adminExtensionPreflight(request: Request): Response | null {
  if (request.method !== "OPTIONS") return null;
  const origin = request.headers.get("origin");
  if (origin !== ADMIN_EXTENSION_ORIGIN)
    return Response.json({ error: "origin not allowed" }, { status: 403 });
  const requestedMethod = request.headers.get("access-control-request-method");
  if (requestedMethod !== "POST")
    return Response.json({ error: "method not allowed" }, { status: 405 });
  const requestedHeaders = (
    request.headers.get("access-control-request-headers") ?? ""
  )
    .split(",")
    .map((header) => header.trim().toLowerCase())
    .filter(Boolean);
  if (requestedHeaders.some((header) => !ALLOWED_REQUEST_HEADERS.has(header)))
    return Response.json(
      { error: "request headers not allowed" },
      { status: 400 },
    );
  return new Response(null, {
    status: 204,
    headers: {
      "access-control-allow-origin": ADMIN_EXTENSION_ORIGIN,
      "access-control-allow-methods": "POST, OPTIONS",
      "access-control-allow-headers": [...ALLOWED_REQUEST_HEADERS].join(", "),
      "access-control-max-age": "600",
      vary: "Origin, Access-Control-Request-Headers",
    },
  });
}
