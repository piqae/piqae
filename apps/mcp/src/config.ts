import { isAbsolute, resolve } from "node:path";

export type JobSubmissionPolicy = "disabled" | "test_only" | "all";

export interface McpConfig {
  apiOrigin: string;
  authorizationServer?: string;
  bindHost: string;
  port: number;
  publicUrl: string;
  staticBearer?: string;
  platformContext?: { workspaceId: string; environmentId: string };
  secretDirectory?: string;
  allowSecretOutput: boolean;
  jobSubmission: JobSubmissionPolicy;
  allowedOrigins: Set<string>;
}

export function loadConfig(
  environment: NodeJS.ProcessEnv = process.env,
): McpConfig {
  const bindHost = environment.PIQAE_MCP_BIND_HOST ?? "127.0.0.1";
  const port = integer(
    environment.PIQAE_MCP_PORT ?? "39300",
    "PIQAE_MCP_PORT",
    1,
    65_535,
  );
  const urlHost = bindHost.includes(":")
    ? `[${bindHost.replace(/^\[|\]$/g, "")}]`
    : bindHost;
  const publicUrl = normalizedUrl(
    environment.PIQAE_MCP_PUBLIC_URL ?? `http://${urlHost}:${port}/mcp`,
    "PIQAE_MCP_PUBLIC_URL",
  );
  const apiOrigin = normalizedUrl(
    environment.PIQAE_API_ORIGIN ?? "https://api.piqae.com",
    "PIQAE_API_ORIGIN",
    true,
  );
  const authorizationServer = optionalUrl(
    environment.PIQAE_MCP_AUTHORIZATION_SERVER,
    "PIQAE_MCP_AUTHORIZATION_SERVER",
    true,
  );
  const secretDirectory = environment.PIQAE_MCP_SECRET_DIRECTORY;
  if (secretDirectory && !isAbsolute(secretDirectory)) {
    throw new Error("PIQAE_MCP_SECRET_DIRECTORY must be an absolute path");
  }

  const workspaceId = environment.PIQAE_WORKSPACE_ID;
  const environmentId = environment.PIQAE_ENVIRONMENT_ID;
  if ((workspaceId === undefined) !== (environmentId === undefined)) {
    throw new Error(
      "PIQAE_WORKSPACE_ID and PIQAE_ENVIRONMENT_ID must be set together",
    );
  }

  const jobSubmission = environment.PIQAE_MCP_JOB_SUBMISSION ?? "disabled";
  if (!["disabled", "test_only", "all"].includes(jobSubmission)) {
    throw new Error(
      "PIQAE_MCP_JOB_SUBMISSION must be disabled, test_only, or all",
    );
  }

  const staticBearer =
    environment.PIQAE_PLATFORM_KEY ??
    environment.PIQAE_ACCESS_TOKEN ??
    environment.PIQAE_API_KEY;
  if (staticBearer?.includes("\n") || staticBearer?.includes("\r")) {
    throw new Error("Piqae bearer credentials cannot contain newlines");
  }

  return {
    apiOrigin,
    ...(authorizationServer ? { authorizationServer } : {}),
    bindHost,
    port,
    publicUrl,
    ...(staticBearer ? { staticBearer } : {}),
    ...(workspaceId && environmentId
      ? { platformContext: { workspaceId, environmentId } }
      : {}),
    ...(secretDirectory ? { secretDirectory: resolve(secretDirectory) } : {}),
    allowSecretOutput: environment.PIQAE_MCP_ALLOW_SECRET_OUTPUT === "true",
    jobSubmission: jobSubmission as JobSubmissionPolicy,
    allowedOrigins: new Set(
      (environment.PIQAE_MCP_ALLOWED_ORIGINS ?? "")
        .split(",")
        .map((value) => value.trim())
        .filter(Boolean),
    ),
  };
}

function integer(
  value: string,
  name: string,
  minimum: number,
  maximum: number,
): number {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(`${name} must be an integer from ${minimum} to ${maximum}`);
  }
  return parsed;
}

function optionalUrl(
  value: string | undefined,
  name: string,
  requireHttps: boolean,
): string | undefined {
  return value === undefined
    ? undefined
    : normalizedUrl(value, name, requireHttps);
}

function normalizedUrl(
  value: string,
  name: string,
  requireHttps = false,
): string {
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error(`${name} must be an absolute URL`);
  }
  if (url.username || url.password || url.search || url.hash) {
    throw new Error(
      `${name} cannot contain credentials, a query, or a fragment`,
    );
  }
  if (requireHttps && url.protocol !== "https:" && !isLoopback(url.hostname)) {
    throw new Error(`${name} must use HTTPS outside loopback development`);
  }
  return url.toString().replace(/\/$/, "");
}

export function isLoopback(hostname: string): boolean {
  const unbracketed = hostname.replace(/^\[|\]$/g, "");
  return (
    unbracketed === "127.0.0.1" ||
    unbracketed === "::1" ||
    unbracketed === "localhost"
  );
}
