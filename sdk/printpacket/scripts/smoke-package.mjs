import assert from "node:assert/strict";
import { mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { pathToFileURL } from "node:url";

const scratch = await mkdtemp(join(tmpdir(), "printpacket-core-smoke-"));

function run(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, encoding: "utf8", stdio: "pipe" });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed\n${result.stdout}\n${result.stderr}`);
  }
}

try {
  run("npm", ["pack", ".", "--pack-destination", scratch], process.cwd());
  const archive = (await readdir(scratch)).find((entry) => entry.endsWith(".tgz"));
  assert.ok(archive, "npm pack did not create a PrintPacket archive");
  await writeFile(
    join(scratch, "package.json"),
    JSON.stringify({ name: "printpacket-smoke", private: true, type: "module" })
  );
  run(
    "npm",
    ["install", "--ignore-scripts", "--no-audit", "--no-fund", join(scratch, archive)],
    scratch
  );

  const packageRoot = join(scratch, "node_modules/@printpacket/core");
  const kit = await import(pathToFileURL(join(packageRoot, "dist/index.js")).href);
  assert.equal(kit.PRINTPACKET_V1, "printpacket/v1");
  assert.equal(
    kit.definePacket({
      format: "printpacket/v1",
      media: { kind: "label", width_mm: 50, height_mm: 30 },
      body: []
    }).format,
    "printpacket/v1"
  );
  const schema = JSON.parse(
    await readFile(join(packageRoot, "schema/printpacket-v1.schema.json"), "utf8")
  );
  assert.equal(schema.title, "PrintPacket v1");
  assert.match(
    await readFile(join(packageRoot, "LICENSE"), "utf8"),
    /TERMS AND CONDITIONS FOR USE/
  );
} finally {
  await rm(scratch, { recursive: true, force: true });
}
