import { readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

const packageRoot = fileURLToPath(new URL("../", import.meta.url));
const repositoryLicense = fileURLToPath(new URL("../../../LICENSE", import.meta.url));
const packageLicense = fileURLToPath(new URL("../LICENSE", import.meta.url));
const expected = await readFile(repositoryLicense, "utf8");

if (process.argv.includes("--check")) {
  const actual = await readFile(packageLicense, "utf8").catch(() => "");
  if (actual !== expected) {
    console.error(`${packageRoot}LICENSE is stale; run pnpm --filter @printpacket/core generate`);
    process.exitCode = 1;
  }
} else {
  await writeFile(packageLicense, expected);
}
