import { once } from "node:events";

import express from "express";
import { afterEach, describe, expect, it } from "vitest";

import { adminExtensionPreflightMiddleware } from "../server/admin-extension-cors.mjs";

let server;

afterEach(async () => {
  if (!server) return;
  server.close();
  await once(server, "close");
  server = undefined;
});

describe("Shopify production server preflight middleware", () => {
  it("answers trusted action preflights before React Router", async () => {
    const app = express();
    app.use(adminExtensionPreflightMiddleware);
    app.all("*", (_request, response) => response.sendStatus(418));
    server = app.listen(0, "127.0.0.1");
    await once(server, "listening");
    const address = server.address();
    if (!address || typeof address === "string")
      throw new Error("Test server did not expose a TCP port");

    const response = await fetch(
      `http://127.0.0.1:${address.port}/api/print/admin/previews`,
      {
        method: "OPTIONS",
        headers: {
          origin: "https://extensions.shopifycdn.com",
          "access-control-request-method": "POST",
          "access-control-request-headers":
            "authorization, content-type, idempotency-key",
        },
      },
    );

    expect(response.status).toBe(204);
    expect(response.headers.get("access-control-allow-origin")).toBe(
      "https://extensions.shopifycdn.com",
    );
    expect(response.headers.get("access-control-allow-headers")).toContain(
      "authorization",
    );
    expect(response.headers.get("access-control-allow-headers")).toContain(
      "idempotency-key",
    );
  });

  it("leaves trusted POST CORS ownership with React Router", async () => {
    const app = express();
    app.use(adminExtensionPreflightMiddleware);
    app.all("*", (_request, response) => {
      response.append("access-control-allow-origin", "*");
      response.sendStatus(418);
    });
    server = app.listen(0, "127.0.0.1");
    await once(server, "listening");
    const address = server.address();
    if (!address || typeof address === "string")
      throw new Error("Test server did not expose a TCP port");

    const response = await fetch(
      `http://127.0.0.1:${address.port}/api/print/admin/previews`,
      {
        method: "POST",
        headers: { origin: "https://extensions.shopifycdn.com" },
      },
    );

    expect(response.status).toBe(418);
    expect(response.headers.get("access-control-allow-origin")).toBe("*");
  });
});
