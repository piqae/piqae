import type { LoaderFunctionArgs } from "react-router";

import { adminExtensionCors } from "../../server/admin-extension-cors.mjs";

export function loader({ request }: LoaderFunctionArgs) {
  const state = new URL(request.url).searchParams.get("state");
  const content =
    state === "loading"
      ? `<main aria-label="Generating document preview" aria-busy="true">
          <div class="page" aria-hidden="true">
            <span class="line wide"></span><span class="line medium"></span>
            <span class="block"></span>
            <span class="line wide"></span><span class="line short"></span>
          </div>
        </main>`
      : state === "error"
        ? `<main><h1>Preview unavailable</h1><p>Check the message beside the preview, then try again.</p></main>`
        : `<main><h1>Preparing preview</h1><p>Choose a published document to preview the selected orders.</p></main>`;
  return adminExtensionCors(
    new Response(
      `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width,initial-scale=1">
    <title>Piqae print preview</title>
    <style>
      :root { color-scheme: light dark; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }
      body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: #f6f6f7; color: #202223; }
      main { max-width: 32rem; padding: 2rem; text-align: center; }
      h1 { font-size: 1.25rem; margin: 0 0 .5rem; }
      p { color: #6d7175; line-height: 1.5; margin: 0; }
      .page { width: min(72vw, 30rem); aspect-ratio: 210 / 297; box-sizing: border-box; padding: 12%; background: #fff; box-shadow: 0 1px 5px #0002; text-align: left; overflow: hidden; }
      .line, .block { display: block; border-radius: .35rem; background: linear-gradient(90deg, #eceef0 25%, #f7f8f8 45%, #eceef0 65%); background-size: 300% 100%; animation: shimmer 1.35s ease-in-out infinite; }
      .line { height: .8rem; margin-bottom: 1rem; }
      .wide { width: 100%; } .medium { width: 62%; } .short { width: 38%; }
      .block { height: 34%; margin: 2rem 0; }
      @keyframes shimmer { from { background-position: 100% 0; } to { background-position: 0 0; } }
      @media (prefers-reduced-motion: reduce) { .line, .block { animation: none; } }
      @media print { body { display: none; } }
    </style>
  </head>
  <body>${content}</body>
</html>`,
      {
        headers: {
          "content-type": "text/html; charset=utf-8",
          "cache-control": "no-store, private",
          "content-security-policy":
            "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; frame-ancestors https://admin.shopify.com https://*.myshopify.com",
          "referrer-policy": "no-referrer",
          "x-content-type-options": "nosniff",
        },
      },
    ),
  );
}
