import { chmod, mkdtemp, readFile, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { loadConfig } from "../src/config.js";
import { assertSecretDeliveryReady, deliverSecret } from "../src/secrets.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(
    temporaryDirectories
      .splice(0)
      .map((path) => rm(path, { recursive: true, force: true })),
  );
});

describe("one-time secret delivery", () => {
  it("creates an owner-only file without returning the secret", async () => {
    const root = await privateTemporaryDirectory();
    const config = loadConfig({ PIQAE_MCP_SECRET_DIRECTORY: root });
    const delivered = await deliverSecret(
      config,
      "api-key",
      "key_123",
      "piq_test_example",
      "file",
    );
    expect(delivered.secret).toBeUndefined();
    expect(delivered.path).toBe(join(root, "api-key-key_123.json"));
    expect((await stat(delivered.path!)).mode & 0o777).toBe(0o600);
    expect(await readFile(delivered.path!, "utf8")).toContain(
      "piq_test_example",
    );
  });

  it("rejects a secret directory visible to group or other users", async () => {
    const root = await privateTemporaryDirectory();
    await chmod(root, 0o755);
    const config = loadConfig({ PIQAE_MCP_SECRET_DIRECTORY: root });
    await expect(assertSecretDeliveryReady(config, "file")).rejects.toThrow(
      /mode 0700/,
    );
  });

  it("requires both server and per-call opt-in for transcript output", async () => {
    await expect(
      deliverSecret(
        loadConfig({}),
        "webhook",
        "wh_123",
        "whsec_example",
        "response",
      ),
    ).rejects.toThrow(/Secret output is disabled/);
    const delivered = await deliverSecret(
      loadConfig({ PIQAE_MCP_ALLOW_SECRET_OUTPUT: "true" }),
      "webhook",
      "wh_123",
      "whsec_example",
      "response",
    );
    expect(delivered.secret).toBe("whsec_example");
  });
});

async function privateTemporaryDirectory(): Promise<string> {
  const path = await mkdtemp(join(tmpdir(), "piqae-mcp-test-"));
  await chmod(path, 0o700);
  temporaryDirectories.push(path);
  return path;
}
