import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const source = fileURLToPath(new URL("../../../standards/printpacket/schema/printpacket-v1.schema.json", import.meta.url));
const target = fileURLToPath(new URL("../schema/printpacket-v1.schema.json", import.meta.url));
const expected = await readFile(source, "utf8");
if (process.argv.includes("--check")) {
  const actual = await readFile(target, "utf8").catch(() => "");
  if (actual !== expected) {
    throw new Error("PrintPacket schema is stale; run pnpm --filter @printpacket/core generate");
  }
} else {
  await writeFile(target, expected);
}
