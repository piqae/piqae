import { PiqaeClient, PiqaeError, PiqaePlatform } from "@piqae/sdk";
import type { McpConfig } from "./config.js";

export interface AuthContext {
  authInfo?: { token: string };
}

export function bearer(config: McpConfig, extra?: AuthContext): string {
  const token = extra?.authInfo?.token ?? config.staticBearer;
  if (!token) {
    throw new Error(
      "No Piqae bearer is available. Configure PIQAE_API_KEY/PIQAE_ACCESS_TOKEN for stdio, or authenticate the Streamable HTTP request.",
    );
  }
  return token;
}

export function tenantClient(
  config: McpConfig,
  extra?: AuthContext,
  platformContext = config.platformContext,
): PiqaeClient {
  const token = bearer(config, extra);
  if (token.startsWith("piq_platform_") || token.startsWith("spl_platform_")) {
    if (!platformContext) {
      throw new Error(
        "Platform credentials require explicit workspace and environment context.",
      );
    }
    return new PiqaeClient({
      baseUrl: config.apiOrigin,
      platformKey: token,
      platformContext,
    });
  }
  if (platformContext) {
    throw new Error(
      "Tenant credentials cannot select platform workspace/environment headers.",
    );
  }
  return new PiqaeClient({
    baseUrl: config.apiOrigin,
    accessToken: () => token,
  });
}

export function platformClient(
  config: McpConfig,
  extra?: AuthContext,
): PiqaePlatform {
  return new PiqaePlatform({
    baseUrl: config.apiOrigin,
    platformKey: bearer(config, extra),
  });
}

export async function rawRequest<T>(
  config: McpConfig,
  extra: AuthContext | undefined,
  method: string,
  path: string,
  body?: unknown,
  platformContext?: { workspaceId: string; environmentId: string },
): Promise<T> {
  const headers: Record<string, string> = {
    accept: "application/json",
    authorization: `Bearer ${bearer(config, extra)}`,
  };
  if (platformContext) {
    headers["x-piqae-workspace-id"] = platformContext.workspaceId;
    headers["x-piqae-environment-id"] = platformContext.environmentId;
  }
  if (body !== undefined) headers["content-type"] = "application/json";
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), 30_000);
  timeout.unref();
  try {
    const response = await fetch(`${config.apiOrigin}${path}`, {
      method,
      headers,
      signal: controller.signal,
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    if (!response.ok) {
      const envelope = (await response.json().catch(() => undefined)) as
        | {
            error?: {
              code?: string;
              message?: string;
              request_id?: string;
              retryable?: boolean;
            };
          }
        | undefined;
      throw new PiqaeError(response.status, {
        code: envelope?.error?.code ?? "unexpected_response",
        message:
          envelope?.error?.message ??
          response.statusText ??
          "Piqae request failed",
        ...(envelope?.error?.request_id
          ? { request_id: envelope.error.request_id }
          : {}),
        ...(envelope?.error?.retryable === undefined
          ? {}
          : { retryable: envelope.error.retryable }),
      });
    }
    if (response.status === 204) return undefined as T;
    return (await response.json()) as T;
  } finally {
    clearTimeout(timeout);
  }
}

export async function verifyBearer(
  config: McpConfig,
  token: string,
): Promise<{
  clientId: string;
  scopes: string[];
  extra: Record<string, unknown>;
}> {
  const headers = {
    accept: "application/json",
    authorization: `Bearer ${token}`,
  };
  const platform =
    token.startsWith("piq_platform_") || token.startsWith("spl_platform_");
  const response = await fetch(
    `${config.apiOrigin}${platform ? "/v1/platform/accounts" : "/v1/identity/me"}`,
    { headers, signal: AbortSignal.timeout(10_000) },
  );
  if (!response.ok) throw new Error("invalid or expired Piqae bearer");
  const jwt = jwtPayload(token);
  if (config.authorizationServer && !audienceIncludes(jwt, config.publicUrl)) {
    throw new Error("OAuth token is not audience-bound to this MCP resource");
  }
  const value = (await response.json()) as
    Record<string, unknown> | Array<unknown>;
  const identity = Array.isArray(value) ? undefined : value;
  return {
    clientId: String(
      jwt?.client_id ??
        jwt?.azp ??
        identity?.id ??
        (platform ? "piqae-platform" : "piqae-identity"),
    ),
    scopes: stringList(jwt?.permissions ?? jwt?.scope),
    extra: {
      credential_type: platform ? "platform" : "tenant",
      ...(identity?.workspace_id
        ? { workspace_id: identity.workspace_id }
        : {}),
      ...(identity?.environment_id
        ? { environment_id: identity.environment_id }
        : {}),
    },
  };
}

function jwtPayload(token: string): Record<string, unknown> | undefined {
  const parts = token.split(".");
  if (parts.length !== 3 || !parts[1]) return undefined;
  try {
    const value = JSON.parse(
      Buffer.from(parts[1], "base64url").toString("utf8"),
    ) as unknown;
    return typeof value === "object" && value !== null && !Array.isArray(value)
      ? (value as Record<string, unknown>)
      : undefined;
  } catch {
    return undefined;
  }
}

function audienceIncludes(
  payload: Record<string, unknown> | undefined,
  resource: string,
): boolean {
  const audience = payload?.aud;
  return (Array.isArray(audience) ? audience : [audience]).some(
    (value) => value === resource,
  );
}

function stringList(value: unknown): string[] {
  if (Array.isArray(value))
    return value.filter((item): item is string => typeof item === "string");
  return typeof value === "string" ? value.split(/\s+/).filter(Boolean) : [];
}
