import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { access, mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const scratch = await mkdtemp(join(tmpdir(), "piqae-mcp-smoke-"));
const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const sdkDirectory = resolve(scriptDirectory, "../../../sdk/typescript");

function run(command, args, cwd) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    stdio: "pipe",
  });
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} failed\n${result.stdout}\n${result.stderr}`,
    );
  }
  return result.stdout;
}

try {
  run("pnpm", ["pack", "--pack-destination", scratch], sdkDirectory);
  run("pnpm", ["pack", "--pack-destination", scratch], process.cwd());
  const archives = await readdir(scratch);
  const sdkArchive = archives.find((entry) => entry.startsWith("piqae-sdk-"));
  const mcpArchive = archives.find((entry) =>
    entry.startsWith("piqae-mcp-server-"),
  );
  assert.ok(sdkArchive, "pnpm pack did not create an SDK archive");
  assert.ok(mcpArchive, "pnpm pack did not create an MCP archive");

  await writeFile(
    join(scratch, "package.json"),
    JSON.stringify({ name: "piqae-mcp-smoke", private: true, type: "module" }),
  );
  run(
    "npm",
    [
      "install",
      "--ignore-scripts",
      "--no-audit",
      "--no-fund",
      join(scratch, sdkArchive),
      join(scratch, mcpArchive),
    ],
    scratch,
  );

  const installed = join(
    scratch,
    "node_modules/@piqae/mcp-server/dist/index.js",
  );
  const mcp = await import(pathToFileURL(installed).href);
  assert.equal(typeof mcp.createPiqaeMcpServer, "function");
  assert.equal(typeof mcp.loadConfig, "function");

  const executable = join(scratch, "node_modules/.bin/piqae-mcp");
  await access(executable);
  const help = run(executable, ["--help"], scratch);
  assert.match(help, /piqae-mcp \[--stdio \| --http\]/);
} finally {
  await rm(scratch, { recursive: true, force: true });
}
