export const ADMIN_EXTENSION_ORIGIN = "https://extensions.shopifycdn.com";

const ALLOWED_REQUEST_HEADERS = new Set([
  "authorization",
  "content-type",
  "idempotency-key",
  "x-requested-with",
]);

const ADMIN_EXTENSION_PRINT_SOURCE_PATHS = new Set([
  "/api/public/print-placeholder",
  "/api/public/previews/artifact",
]);

export function adminExtensionCors(response) {
  response.headers.set("access-control-allow-origin", ADMIN_EXTENSION_ORIGIN);
  response.headers.append("vary", "Origin");
  return response;
}

export function isAdminExtensionPrintSourcePath(pathname) {
  return ADMIN_EXTENSION_PRINT_SOURCE_PATHS.has(pathname);
}

export function isAdminExtensionPreflightPath(pathname) {
  return (
    isAdminExtensionPrintSourcePath(pathname) ||
    pathname === "/api/print/admin" ||
    pathname === "/api/print/admin-drafts" ||
    pathname === "/api/print/admin/readiness" ||
    pathname === "/api/print/admin/previews" ||
    /^\/api\/print\/previews\/[^/]+\/(approve|cancel)$/.test(pathname)
  );
}

export function adminExtensionPreflight(request) {
  if (request.method !== "OPTIONS") return null;
  const origin = request.headers.get("origin");
  if (origin !== ADMIN_EXTENSION_ORIGIN)
    return Response.json({ error: "origin not allowed" }, { status: 403 });
  const requestedMethod = request.headers.get("access-control-request-method");
  const allowedMethod = isAdminExtensionPrintSourcePath(
    new URL(request.url).pathname,
  )
    ? "GET"
    : "POST";
  if (requestedMethod !== allowedMethod)
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
      "access-control-allow-methods": `${allowedMethod}, OPTIONS`,
      "access-control-allow-headers": [...ALLOWED_REQUEST_HEADERS].join(", "),
      "access-control-max-age": "600",
      vary: "Origin, Access-Control-Request-Headers",
    },
  });
}

export async function adminExtensionPreflightMiddleware(
  request,
  response,
  next,
) {
  if (!isAdminExtensionPreflightPath(request.path)) {
    next();
    return;
  }
  if (request.method !== "OPTIONS") {
    // Authenticated React Router actions use Shopify's `cors()` helper, while
    // public print-source loaders add the same trusted origin explicitly.
    // Adding another header here produces two Access-Control-Allow-Origin
    // values, which browsers reject before the extension can read the response.
    next();
    return;
  }
  try {
    const headers = new Headers();
    for (const [name, value] of Object.entries(request.headers)) {
      if (value !== undefined)
        headers.set(name, Array.isArray(value) ? value.join(", ") : value);
    }
    const preflight = adminExtensionPreflight(
      new Request(`https://shopify.piqae.com${request.originalUrl}`, {
        method: "OPTIONS",
        headers,
      }),
    );
    if (!preflight) {
      next();
      return;
    }
    response.status(preflight.status);
    preflight.headers.forEach((value, name) => response.setHeader(name, value));
    const body = await preflight.text();
    if (body) response.send(body);
    else response.end();
  } catch (error) {
    next(error);
  }
}
