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
const migration3 = await readFile(
  path.join(root, "migrations/0003_render_execution_policy.sql"),
  "utf8",
);
const migration4 = await readFile(
  path.join(root, "migrations/0004_managed_piqae_accounts.sql"),
  "utf8",
);
const migration5 = await readFile(
  path.join(root, "migrations/0005_template_targets_and_media.sql"),
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
    "INSERT INTO shopify_merchant_settings(shop,retention_days,render_execution_policy) VALUES($1,30,'require_node')",
    ["alpha.myshopify.com"],
  );
  const settings = await client.query(
    "SELECT render_execution_policy FROM shopify_merchant_settings WHERE shop=$1",
    ["alpha.myshopify.com"],
  );
  if (settings.rows[0]?.render_execution_policy !== "require_node")
    throw new Error("render execution policy was not retained");
  await rejects(
    client,
    "UPDATE shopify_merchant_settings SET render_execution_policy='unsafe' WHERE shop=$1",
    ["alpha.myshopify.com"],
    "23514",
  );
  const link = await client.query(
    "SELECT 1 FROM shopify_shop_links WHERE shop=$1",
    ["alpha.myshopify.com"],
  );
  if (link.rowCount !== 0)
    throw new Error("settings unexpectedly created a Piqae link");
  await client.query(
    "INSERT INTO shopify_shop_links(shop,piqae_account_id,encrypted_credential,template_revision_id,entitlement_mode,plan_handle,piqae_live_environment_id,piqae_test_environment_id) VALUES($1,$2,NULL,$3,'shopify_child','development',$4,$5)",
    [
      "alpha.myshopify.com",
      "acct_alpha",
      "rev_alpha",
      "env_live_alpha",
      "env_test_alpha",
    ],
  );
  const managed = await client.query(
    "SELECT encrypted_credential,piqae_live_environment_id,piqae_test_environment_id FROM shopify_shop_links WHERE shop=$1",
    ["alpha.myshopify.com"],
  );
  if (
    managed.rows[0]?.encrypted_credential !== null ||
    managed.rows[0]?.piqae_live_environment_id !== "env_live_alpha" ||
    managed.rows[0]?.piqae_test_environment_id !== "env_test_alpha"
  )
    throw new Error("managed Piqae account context was not retained");
  await rejects(
    client,
    "INSERT INTO shopify_shop_links(shop,piqae_account_id,encrypted_credential,template_revision_id,entitlement_mode,plan_handle) VALUES($1,$2,NULL,$3,'shopify_child','development')",
    ["beta.myshopify.com", "acct_beta", "rev_beta"],
    "23514",
  );

  const templateId = "11111111-1111-4111-8111-111111111111";
  await client.query(
    "INSERT INTO shopify_workflow_templates(id,shop,name,kind,page_size,state,source) VALUES($1,$2,'Invoice','invoice','A4','published',$3)",
    [
      templateId,
      "alpha.myshopify.com",
      '{"schema":"piqae.shopify-printpacket-template/v1","document":{"format":"printpacket/v1","media":{"kind":"paged","size":"a4","margins":{"top_mm":10,"right_mm":10,"bottom_mm":10,"left_mm":10}},"theme":{"font_size_pt":10,"line_height":1.25,"text_color":{"red":0,"green":0,"blue":0}},"resources":{},"body":[]},"editor":{"mode":"visual","liquid":"","roundTrip":"lossless","warnings":[]},"assets":[]}',
    ],
  );
  await client.query(
    "UPDATE shopify_workflow_templates SET kind='label',page_size='100x50mm',design_target_id=$1,design_specification_revision=$2 WHERE shop=$3 AND id=$4",
    ["tgt_alpha", "spec_alpha", "alpha.myshopify.com", templateId],
  );
  const association = await client.query(
    "SELECT kind,page_size,design_target_id,design_specification_revision FROM shopify_workflow_templates WHERE shop=$1 AND id=$2",
    ["alpha.myshopify.com", templateId],
  );
  if (
    association.rows[0]?.kind !== "label" ||
    association.rows[0]?.page_size !== "100x50mm" ||
    association.rows[0]?.design_target_id !== "tgt_alpha" ||
    association.rows[0]?.design_specification_revision !== "spec_alpha"
  )
    throw new Error("template target and media association was not retained");
  await client.query(
    "INSERT INTO shopify_workflow_template_revisions(template_id,shop,revision,name,kind,page_size,source,design_target_id,design_specification_revision) VALUES($1,$2,1,'Product label','label','100x50mm',$3,$4,$5)",
    [templateId, "alpha.myshopify.com", "{}", "tgt_alpha", "spec_alpha"],
  );
  const revision = await client.query(
    "SELECT design_target_id,design_specification_revision FROM shopify_workflow_template_revisions WHERE shop=$1 AND template_id=$2 AND revision=1",
    ["alpha.myshopify.com", templateId],
  );
  if (
    revision.rows[0]?.design_target_id !== "tgt_alpha" ||
    revision.rows[0]?.design_specification_revision !== "spec_alpha"
  )
    throw new Error("published template target association was not retained");
  await rejects(
    client,
    "UPDATE shopify_workflow_templates SET design_target_id=$1,design_specification_revision=NULL WHERE shop=$2 AND id=$3",
    ["tgt_incomplete", "alpha.myshopify.com", templateId],
    "23514",
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
      await client.query(migration3);
      await client.query(migration4);
      if (index === 1) {
        await client.query(
          "INSERT INTO shopify_workflow_templates(id,shop,name,kind,page_size,state,source) VALUES($1,$2,'Existing invoice','invoice','A4','draft','{}')",
          ["33333333-3333-4333-8333-333333333333", "existing.myshopify.com"],
        );
      }
      await client.query(migration5);
      await client.query(migration5);
      if (index === 1) {
        const retained = await client.query(
          "SELECT state FROM shopify_installations WHERE shop=$1",
          ["existing.myshopify.com"],
        );
        if (retained.rows[0]?.state !== "installed")
          throw new Error("N-1 installation was not retained");
        const columns = await client.query(
          "SELECT design_target_id,design_specification_revision FROM shopify_workflow_templates WHERE shop=$1 AND id=$2",
          ["existing.myshopify.com", "33333333-3333-4333-8333-333333333333"],
        );
        if (
          columns.rowCount !== 1 ||
          columns.rows[0]?.design_target_id !== null ||
          columns.rows[0]?.design_specification_revision !== null
        )
          throw new Error("N-1 template association was not retained safely");
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
