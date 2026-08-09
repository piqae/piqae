import {
  createServer,
  type IncomingMessage,
  type ServerResponse,
} from "node:http";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import type { StreamableHTTPServerTransportOptions } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import type { Transport } from "@modelcontextprotocol/sdk/shared/transport.js";
import type { AuthInfo } from "@modelcontextprotocol/sdk/server/auth/types.js";
import type { McpConfig } from "./config.js";
import { isLoopback } from "./config.js";
import { verifyBearer } from "./api.js";
import { createPiqaeMcpServer } from "./server.js";

const MAX_MCP_BODY_BYTES = 1024 * 1024;

export async function startHttpServer(
  config: McpConfig,
): Promise<() => Promise<void>> {
  const publicUrl = new URL(config.publicUrl);
  if (!isLoopback(publicUrl.hostname) && !config.authorizationServer) {
    throw new Error(
      "Remote HTTP requires PIQAE_MCP_AUTHORIZATION_SERVER so clients can perform standards-based OAuth discovery.",
    );
  }
  if (!isLoopback(publicUrl.hostname) && publicUrl.protocol !== "https:") {
    throw new Error("Remote PIQAE_MCP_PUBLIC_URL must use HTTPS.");
  }

  const server = createServer((request, response) => {
    void handleRequest(config, publicUrl, request, response).catch(
      (error: unknown) => {
        console.error(
          `Piqae MCP HTTP failure: ${error instanceof Error ? error.message : "unknown error"}`,
        );
        if (!response.headersSent)
          json(response, 500, { error: "internal_server_error" });
        else response.end();
      },
    );
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(config.port, config.bindHost, () => {
      server.off("error", reject);
      resolve();
    });
  });
  server.headersTimeout = 10_000;
  server.requestTimeout = 15_000;
  server.keepAliveTimeout = 5_000;
  server.maxRequestsPerSocket = 100;
  console.error(`Piqae MCP Streamable HTTP listening at ${config.publicUrl}`);
  return () =>
    new Promise<void>((resolve, reject) => {
      server.close((error) => (error ? reject(error) : resolve()));
    });
}

async function handleRequest(
  config: McpConfig,
  publicUrl: URL,
  request: IncomingMessage,
  response: ServerResponse,
): Promise<void> {
  const requestUrl = new URL(request.url ?? "/", publicUrl.origin);
  const metadataPaths = new Set([
    "/.well-known/oauth-protected-resource",
    `/.well-known/oauth-protected-resource${publicUrl.pathname === "/" ? "" : publicUrl.pathname}`,
  ]);
  if (!validHostAndOrigin(config, publicUrl, request)) {
    json(response, 403, { error: "invalid_host_or_origin" });
    return;
  }
  if (request.method === "GET" && metadataPaths.has(requestUrl.pathname)) {
    if (!config.authorizationServer) {
      json(response, 404, { error: "oauth_metadata_not_configured" });
      return;
    }
    json(response, 200, {
      resource: config.publicUrl,
      authorization_servers: [config.authorizationServer],
      scopes_supported: [
        "api_keys_read",
        "api_keys_write",
        "agents_read",
        "agents_write",
        "printers_read",
        "printers_write",
        "jobs_read",
        "jobs_write",
        "webhooks_read",
        "webhooks_write",
        "usage_read",
        "audit_read",
      ],
      resource_name: "Piqae administration MCP",
      bearer_methods_supported: ["header"],
      resource_documentation: "https://piqae.com/docs",
    });
    return;
  }
  if (requestUrl.pathname !== publicUrl.pathname) {
    json(response, 404, { error: "not_found" });
    return;
  }
  if (request.method !== "POST") {
    response.setHeader("Allow", "POST");
    json(response, 405, { error: "method_not_allowed" });
    return;
  }
  const contentLength = Number(request.headers["content-length"] ?? "0");
  if (
    !Number.isFinite(contentLength) ||
    contentLength < 0 ||
    contentLength > MAX_MCP_BODY_BYTES
  ) {
    json(response, 413, { error: "request_too_large" });
    return;
  }
  if (
    !String(request.headers["content-type"] ?? "")
      .toLowerCase()
      .startsWith("application/json")
  ) {
    json(response, 415, { error: "content_type_must_be_application_json" });
    return;
  }

  const token = authorizationToken(request.headers.authorization);
  if (!token) {
    unauthorized(config, publicUrl, response, "invalid_token");
    return;
  }
  let verified: Awaited<ReturnType<typeof verifyBearer>>;
  try {
    verified = await verifyBearer(config, token);
  } catch {
    unauthorized(config, publicUrl, response, "invalid_token");
    return;
  }

  const auth: AuthInfo = {
    token,
    clientId: verified.clientId,
    scopes: verified.scopes,
    resource: new URL(config.publicUrl),
    extra: verified.extra,
  };
  const authenticatedRequest = request as IncomingMessage & { auth?: AuthInfo };
  authenticatedRequest.auth = auth;
  const transport = new StreamableHTTPServerTransport({
    sessionIdGenerator: undefined,
    enableJsonResponse: true,
  } as unknown as StreamableHTTPServerTransportOptions);
  const mcp = createPiqaeMcpServer(config);
  // The SDK's v1 transport declaration predates exactOptionalPropertyTypes,
  // while its runtime implements the same Transport interface.
  await mcp.connect(transport as unknown as Transport);
  try {
    let body: unknown;
    try {
      body = await readJsonBody(authenticatedRequest);
    } catch (error) {
      const tooLarge = error instanceof RequestBodyError && error.tooLarge;
      json(response, tooLarge ? 413 : 400, {
        error: tooLarge ? "request_too_large" : "invalid_json",
      });
      return;
    }
    await transport.handleRequest(authenticatedRequest, response, body);
  } finally {
    await mcp.close().catch(() => undefined);
  }
}

class RequestBodyError extends Error {
  constructor(readonly tooLarge: boolean) {
    super(tooLarge ? "request too large" : "invalid JSON");
  }
}

async function readJsonBody(request: IncomingMessage): Promise<unknown> {
  const chunks: Buffer[] = [];
  let bytes = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk)
      ? chunk
      : Buffer.from(chunk as Uint8Array);
    bytes += buffer.byteLength;
    if (bytes > MAX_MCP_BODY_BYTES) throw new RequestBodyError(true);
    chunks.push(buffer);
  }
  try {
    return JSON.parse(Buffer.concat(chunks, bytes).toString("utf8")) as unknown;
  } catch {
    throw new RequestBodyError(false);
  }
}

function validHostAndOrigin(
  config: McpConfig,
  publicUrl: URL,
  request: IncomingMessage,
): boolean {
  if (request.headers.host !== publicUrl.host) return false;
  const origin = request.headers.origin;
  if (!origin) return true;
  return origin === publicUrl.origin || config.allowedOrigins.has(origin);
}

function authorizationToken(value: string | undefined): string | undefined {
  if (!value?.startsWith("Bearer ")) return undefined;
  const token = value.slice("Bearer ".length);
  return token && !/\s/.test(token) ? token : undefined;
}

function unauthorized(
  config: McpConfig,
  publicUrl: URL,
  response: ServerResponse,
  code: string,
): void {
  const metadataUrl = new URL(
    `/.well-known/oauth-protected-resource${publicUrl.pathname === "/" ? "" : publicUrl.pathname}`,
    publicUrl.origin,
  );
  const parts = [`Bearer error="${code}"`];
  if (config.authorizationServer)
    parts.push(`resource_metadata="${metadataUrl.toString()}"`);
  response.setHeader("WWW-Authenticate", parts.join(", "));
  json(response, 401, { error: code });
}

function json(
  response: ServerResponse,
  status: number,
  body: Record<string, unknown>,
): void {
  response.statusCode = status;
  response.setHeader("Content-Type", "application/json");
  response.setHeader("Cache-Control", "no-store");
  response.setHeader("X-Content-Type-Options", "nosniff");
  response.end(JSON.stringify(body));
}
