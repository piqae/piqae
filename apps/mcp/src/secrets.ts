import { randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { lstat, mkdir, open, unlink } from "node:fs/promises";
import { join } from "node:path";
import type { McpConfig } from "./config.js";

export type SecretDelivery = "file" | "response";

export interface DeliveredSecret {
  delivery: SecretDelivery;
  path?: string;
  secret?: string;
}

export async function assertSecretDeliveryReady(
  config: McpConfig,
  delivery: SecretDelivery,
): Promise<void> {
  if (delivery === "response") {
    if (!config.allowSecretOutput) {
      throw new Error(
        "Secret output is disabled. Configure PIQAE_MCP_SECRET_DIRECTORY and use delivery=file, or explicitly set PIQAE_MCP_ALLOW_SECRET_OUTPUT=true.",
      );
    }
    return;
  }
  if (!config.secretDirectory) {
    throw new Error("delivery=file requires PIQAE_MCP_SECRET_DIRECTORY");
  }
  await mkdir(config.secretDirectory, { recursive: true, mode: 0o700 });
  const directory = await lstat(config.secretDirectory);
  if (!directory.isDirectory() || directory.isSymbolicLink()) {
    throw new Error(
      "PIQAE_MCP_SECRET_DIRECTORY must be a real directory, not a symlink.",
    );
  }
  // Windows security is enforced by NTFS ACLs; fs.Stats.mode does not
  // represent those access-control entries.
  if (process.platform !== "win32" && (directory.mode & 0o077) !== 0) {
    throw new Error(
      "PIQAE_MCP_SECRET_DIRECTORY must not grant group or other permissions (use mode 0700).",
    );
  }
  if (
    typeof process.getuid === "function" &&
    directory.uid !== process.getuid()
  ) {
    throw new Error(
      "PIQAE_MCP_SECRET_DIRECTORY must be owned by the MCP process user.",
    );
  }
  const probe = join(
    config.secretDirectory,
    `.piqae-mcp-write-probe-${randomUUID()}`,
  );
  const handle = await open(
    probe,
    constants.O_CREAT |
      constants.O_EXCL |
      constants.O_WRONLY |
      constants.O_NOFOLLOW,
    0o600,
  );
  await handle.close();
  await unlink(probe);
}

export async function deliverSecret(
  config: McpConfig,
  kind: "api-key" | "webhook" | "enrolment" | "platform",
  identifier: string,
  secret: string,
  delivery: SecretDelivery,
): Promise<DeliveredSecret> {
  if (delivery === "response") {
    await assertSecretDeliveryReady(config, delivery);
    return { delivery, secret };
  }
  await assertSecretDeliveryReady(config, delivery);
  const secretDirectory = config.secretDirectory;
  if (!secretDirectory) throw new Error("secret directory was not configured");

  const safeIdentifier = identifier
    .replace(/[^A-Za-z0-9_.-]/g, "_")
    .slice(0, 120);
  const path = join(secretDirectory, `${kind}-${safeIdentifier}.json`);
  // O_EXCL prevents replacing an existing entry and O_NOFOLLOW rejects a
  // final-component symlink. The validated 0700, same-UID directory is the
  // boundary because portable Node does not expose openat(2).
  const handle = await open(
    path,
    constants.O_CREAT |
      constants.O_EXCL |
      constants.O_WRONLY |
      constants.O_NOFOLLOW,
    0o600,
  );
  try {
    await handle.writeFile(
      `${JSON.stringify({ kind, id: identifier, secret, created_at: new Date().toISOString() }, null, 2)}\n`,
      { encoding: "utf8" },
    );
  } catch (error) {
    await handle.close().catch(() => undefined);
    await unlink(path).catch(() => undefined);
    throw error;
  }
  await handle.close();
  return { delivery, path };
}
