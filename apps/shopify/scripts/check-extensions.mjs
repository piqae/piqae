import { readFile, stat } from "node:fs/promises";
import { resolve } from "node:path";

import { build } from "esbuild";

const root = resolve(import.meta.dirname, "..");
const extensionConfigs = [
  "extensions/admin-order-print/shopify.extension.toml",
  "extensions/admin-bulk-print/shopify.extension.toml",
  "extensions/admin-order-browser-print/shopify.extension.toml",
  "extensions/admin-bulk-browser-print/shopify.extension.toml",
  "extensions/pos-print/shopify.extension.toml",
  "extensions/customer-order-document/shopify.extension.toml",
  "extensions/admin-draft-print/shopify.extension.toml",
  "extensions/admin-draft-bulk-print/shopify.extension.toml",
  "extensions/admin-product-print/shopify.extension.toml",
  "extensions/admin-product-browser-print/shopify.extension.toml",
];
const allowedTargets = new Set([
  "admin.order-details.action.render",
  "admin.order-index.selection-action.render",
  "admin.order-details.print-action.render",
  "admin.order-index.selection-print-action.render",
  "pos.order-details.action.menu-item.render",
  "pos.order-details.action.render",
  "pos.purchase.post.action.menu-item.render",
  "pos.purchase.post.action.render",
  "pos.home.tile.render",
  "pos.home.modal.render",
  "customer-account.order-status.block.render",
  "admin.draft-order-details.action.render",
  "admin.draft-order-index.selection-action.render",
  "admin.product-details.action.render",
  "admin.product-index.selection-action.render",
  "admin.product-variant-details.action.render",
  "admin.product-details.print-action.render",
  "admin.product-index.selection-print-action.render",
  "pos.product-details.action.menu-item.render",
  "pos.product-details.action.render",
]);

const entryPoints = [];
for (const configPath of extensionConfigs) {
  const absoluteConfig = resolve(root, configPath);
  const config = await readFile(absoluteConfig, "utf8");
  if (!/^api_version = "2026-07"$/m.test(config)) {
    throw new Error(`${configPath} must pin Shopify API 2026-07`);
  }
  const modules = [...config.matchAll(/^module = "(.+)"$/gm)].map(
    (match) => match[1],
  );
  const targets = [...config.matchAll(/^target = "(.+)"$/gm)].map(
    (match) => match[1],
  );
  if (modules.length === 0 || modules.length !== targets.length) {
    throw new Error(`${configPath} must pair every target with one module`);
  }
  const hasNativePrintTarget = targets.some((target) =>
    target.includes("print-action.render"),
  );
  const hasNonPrintTarget = targets.some(
    (target) => !target.includes("print-action.render"),
  );
  if (hasNativePrintTarget && hasNonPrintTarget) {
    throw new Error(
      `${configPath} must keep Shopify native print targets in a separate extension`,
    );
  }
  for (const target of targets) {
    if (!allowedTargets.has(target))
      throw new Error(`${configPath} uses unreviewed target ${target}`);
  }
  for (const modulePath of modules) {
    const entryPoint = resolve(absoluteConfig, "..", modulePath);
    await stat(entryPoint);
    entryPoints.push(entryPoint);
  }
}

const result = await build({
  entryPoints,
  bundle: true,
  format: "esm",
  jsx: "automatic",
  jsxImportSource: "preact",
  minify: true,
  platform: "browser",
  outdir: "out",
  write: false,
});

for (const output of result.outputFiles) {
  if (output.contents.byteLength > 64 * 1024) {
    throw new Error(
      `${output.path} is ${output.contents.byteLength} bytes; Shopify permits 64 KB`,
    );
  }
}

console.log(
  `Validated ${extensionConfigs.length} extension configs and ${result.outputFiles.length} bundles`,
);
