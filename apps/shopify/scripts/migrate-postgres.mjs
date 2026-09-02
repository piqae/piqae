#!/usr/bin/env node
import { readFile, readdir } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import pg from "pg";

const databaseUrl = process.env.DATABASE_URL;
if (!databaseUrl) {
  throw new Error("DATABASE_URL is required to apply Shopify migrations");
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const migrationsDirectory = path.join(root, "migrations");
const migrationFiles = (await readdir(migrationsDirectory))
  .filter((filename) => filename.endsWith(".sql"))
  .sort();
if (!migrationFiles.length) throw new Error("No Shopify migrations found");
for (const [index, filename] of migrationFiles.entries()) {
  if (!/^\d{4}_[a-z0-9_]+\.sql$/.test(filename))
    throw new Error(`Invalid Shopify migration filename: ${filename}`);
  const expectedPrefix = String(index + 1).padStart(4, "0");
  if (!filename.startsWith(`${expectedPrefix}_`))
    throw new Error(
      `Shopify migration sequence is not contiguous at ${expectedPrefix}`,
    );
}
const client = new pg.Client({
  connectionString: databaseUrl,
  connectionTimeoutMillis: 10_000,
  statement_timeout: 30_000,
});

try {
  await client.connect();
  await client.query("BEGIN");
  await client.query("SELECT pg_advisory_xact_lock($1)", [1_347_614_341]);
  for (const filename of migrationFiles) {
    const sql = await readFile(
      path.join(migrationsDirectory, filename),
      "utf8",
    );
    await client.query(sql);
  }
  await client.query("COMMIT");
  console.log(`Applied ${migrationFiles.length} Shopify migrations`);
} catch (error) {
  await client.query("ROLLBACK").catch(() => undefined);
  throw error;
} finally {
  await client.end().catch(() => undefined);
}
