#!/usr/bin/env node
import { randomBytes } from "node:crypto";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";
import pg from "pg";

const databaseUrl = process.env.PIQAE_TEST_DATABASE_URL;
if (!databaseUrl) {
  const message =
    "SKIP Shopify PostgreSQL migrations: PIQAE_TEST_DATABASE_URL is not set";
  if (process.env.PIQAE_REQUIRE_POSTGRES_TESTS === "1") {
    console.error(message);
    process.exit(1);
  }
  console.log(message);
  process.exit(0);
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const migration1 = await readFile(
  path.join(root, "migrations/0001_shopify_core.sql"),
  "utf8",
);
const migration2 = await readFile(
  path.join(root, "migrations/0002_merchant_workflows.sql"),
  "utf8",
);
const suffix = randomBytes(8).toString("hex");
const schemas = [
  `piqae_shopify_fresh_${suffix}`,
  `piqae_shopify_upgrade_${suffix}`,
];
const pool = new pg.Pool({
  connectionString: databaseUrl,
  max: 1,
  statement_timeout: 15_000,
});

function identifier(value) {
  if (!/^piqae_shopify_(?:fresh|upgrade)_[a-f0-9]{16}$/.test(value))
    throw new Error("refusing unsafe schema identifier");
  return `"${value}"`;
}

async function rejects(client, sql, params, expectedCode) {
  try {
    await client.query(sql, params);
  } catch (error) {
    if (error?.code === expectedCode) return;
    throw error;
  }
  throw new Error(`expected PostgreSQL error ${expectedCode}`);
}

async function assertions(client) {
  await client.query(
    "INSERT INTO shopify_installations(shop,state) VALUES ($1,'installed'),($2,'installed')",
    ["alpha.myshopify.com", "beta.myshopify.com"],
  );
  await rejects(
    client,
    "INSERT INTO shopify_installations(shop,state) VALUES ($1,'installed')",
    ["https://invalid.myshopify.com"],
    "23514",
  );
  await client.query(
    "INSERT INTO shopify_merchant_settings(shop,retention_days) VALUES($1,30)",
    ["alpha.myshopify.com"],
  );
  const link = await client.query(
    "SELECT 1 FROM shopify_shop_links WHERE shop=$1",
    ["alpha.myshopify.com"],
  );
  if (link.rowCount !== 0)
    throw new Error("settings unexpectedly created a Piqae link");

  const templateId = "11111111-1111-4111-8111-111111111111";
  await client.query(
    "INSERT INTO shopify_workflow_templates(id,shop,name,kind,page_size,state,source) VALUES($1,$2,'Invoice','invoice','A4','published',$3)",
    [
      templateId,
      "alpha.myshopify.com",
      '{"schema":"piqae.shopify-business-template/v1","document":{"format":"piqae.business-document/v1","media":{"kind":"paged","size":"a4","margins":{"top_mm":10,"right_mm":10,"bottom_mm":10,"left_mm":10}},"theme":{"font_size_pt":10,"line_height":1.25,"text_color":{"red":0,"green":0,"blue":0}},"resources":{},"body":[]},"editor":{"mode":"visual","liquid":"","roundTrip":"lossless","warnings":[]},"assets":[]}',
    ],
  );
  const crossRead = await client.query(
    "SELECT 1 FROM shopify_workflow_templates WHERE shop=$1 AND id=$2",
    ["beta.myshopify.com", templateId],
  );
  if (crossRead.rowCount !== 0)
    throw new Error("cross-tenant template probe leaked data");
  await rejects(
    client,
    "INSERT INTO shopify_automation_rules(id,shop,name,trigger_event,delivery,template_id,destination) VALUES($1,$2,'Cross tenant','order_paid','printer',$3,'printer_1')",
    ["22222222-2222-4222-8222-222222222222", "beta.myshopify.com", templateId],
    "23503",
  );
}

try {
  for (const [index, schema] of schemas.entries()) {
    const client = await pool.connect();
    try {
      await client.query(`CREATE SCHEMA ${identifier(schema)}`);
      await client.query(`SET search_path TO ${identifier(schema)}, public`);
      await client.query(migration1);
      if (index === 1) {
        await client.query(
          "INSERT INTO shopify_installations(shop,state) VALUES($1,'installed')",
          ["existing.myshopify.com"],
        );
      }
      await client.query(migration2);
      if (index === 1) {
        const retained = await client.query(
          "SELECT state FROM shopify_installations WHERE shop=$1",
          ["existing.myshopify.com"],
        );
        if (retained.rows[0]?.state !== "installed")
          throw new Error("N-1 installation was not retained");
      }
      await assertions(client);
    } finally {
      client.release();
    }
  }
  console.log(
    "PASS Shopify PostgreSQL migrations: fresh, N-1, tenant isolation",
  );
} finally {
  const cleanup = await pool.connect();
  try {
    for (const schema of schemas)
      await cleanup.query(
        `DROP SCHEMA IF EXISTS ${identifier(schema)} CASCADE`,
      );
  } finally {
    cleanup.release();
    await pool.end();
  }
}
