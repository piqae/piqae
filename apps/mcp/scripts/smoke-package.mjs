import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtemp, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const scratch = await mkdtemp(join(tmpdir(), "piqae-mcp-smoke-"));

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
  run("pnpm", ["pack", "--pack-destination", scratch], process.cwd());
  const archive = (await readdir(scratch)).find((entry) =>
    entry.endsWith(".tgz"),
  );
  assert.ok(archive, "pnpm pack did not create an MCP archive");

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
      join(scratch, archive),
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

  const help = run(
    "node",
    [join(scratch, "node_modules/@piqae/mcp-server/dist/index.js"), "--help"],
    scratch,
  );
  assert.match(help, /piqae-mcp \[--stdio \| --http\]/);
} finally {
  await rm(scratch, { recursive: true, force: true });
}
