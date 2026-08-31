import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { createRequestHandler } from "@react-router/express";
import express from "express";

import { adminExtensionPreflightMiddleware } from "./server/admin-extension-cors.mjs";

process.env.NODE_ENV ??= "production";

const root = path.dirname(fileURLToPath(import.meta.url));
const build = await import(
  pathToFileURL(path.join(root, "build/server/index.js")).href
);
const app = express();
app.disable("x-powered-by");

app.use(adminExtensionPreflightMiddleware);

const client = path.join(root, "build/client");
app.use(
  "/assets",
  express.static(path.join(client, "assets"), {
    immutable: true,
    maxAge: "1y",
  }),
);
app.use(express.static(client, { maxAge: "1h" }));
app.use(express.static(path.join(root, "public"), { maxAge: "1h" }));
app.all(
  "*",
  createRequestHandler({
    build,
    mode: process.env.NODE_ENV,
  }),
);
app.use((error, _request, response, _next) => {
  console.error("[piqae-shopify] server middleware failed", {
    kind: error instanceof Error ? error.name : typeof error,
  });
  if (response.headersSent) {
    _next(error);
    return;
  }
  response.status(500).json({ message: "Unexpected Server Error" });
});

const port = Number(process.env.PORT ?? 3000);
if (!Number.isInteger(port) || port < 1 || port > 65_535)
  throw new Error("PORT must be an integer between 1 and 65535");
const host = process.env.HOST || "0.0.0.0";
const server = app.listen(port, host, () => {
  console.log(`[piqae-shopify] listening on ${host}:${port}`);
});

for (const signal of ["SIGTERM", "SIGINT"]) {
  process.once(signal, () =>
    server.close((error) => error && console.error(error)),
  );
}
