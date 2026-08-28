#!/usr/bin/env node
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import pg from "pg";

const databaseUrl = process.env.DATABASE_URL;
if (!databaseUrl) {
  throw new Error("DATABASE_URL is required to apply Shopify migrations");
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const migrationFiles = [
  "0001_shopify_core.sql",
  "0002_merchant_workflows.sql",
  "0003_render_execution_policy.sql",
  "0004_managed_piqae_accounts.sql",
  "0005_template_targets_and_media.sql",
];
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
    const sql = await readFile(path.join(root, "migrations", filename), "utf8");
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
