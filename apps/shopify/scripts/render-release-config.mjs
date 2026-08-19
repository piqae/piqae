#!/usr/bin/env node

import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { parse } from "smol-toml";

function required(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required`);
  return value;
}

const clientId = required("SHOPIFY_CLIENT_ID");
const appUrl = new URL(required("SHOPIFY_APP_URL"));
if (appUrl.protocol !== "https:")
  throw new Error("SHOPIFY_APP_URL must use HTTPS");
if (appUrl.username || appUrl.password || appUrl.search || appUrl.hash)
  throw new Error("SHOPIFY_APP_URL must be a clean HTTPS origin");
if (appUrl.pathname !== "/" && appUrl.pathname !== "")
  throw new Error("SHOPIFY_APP_URL must not contain a path");

const source = resolve("shopify.app.toml");
const output = resolve("shopify.app.release.toml");
let config = await readFile(source, "utf8");

function replaceExactlyOnce(target, replacement, label) {
  const occurrences = config.split(target).length - 1;
  if (occurrences !== 1)
    throw new Error(`expected exactly one ${label}; found ${occurrences}`);
  config = config.replace(target, replacement);
}

replaceExactlyOnce(
  'client_id = "development-client-id"',
  `client_id = ${JSON.stringify(clientId)}`,
  "placeholder client_id",
);
replaceExactlyOnce(
  'application_url = "https://example.invalid"',
  `application_url = ${JSON.stringify(appUrl.origin)}`,
  "placeholder application_url",
);
replaceExactlyOnce(
  'redirect_urls = [ "https://example.invalid/auth/callback" ]',
  `redirect_urls = [ ${JSON.stringify(`${appUrl.origin}/auth/callback`)} ]`,
  "placeholder redirect_urls",
);
replaceExactlyOnce(
  'url = "https://example.invalid"',
  `url = ${JSON.stringify(appUrl.origin)}`,
  "placeholder app-proxy URL",
);

if (
  config.includes("example.invalid") ||
  config.includes("development-client-id")
)
  throw new Error("release configuration still contains placeholders");

const parsed = parse(config);
if (parsed.client_id !== clientId)
  throw new Error("rendered client_id does not match SHOPIFY_CLIENT_ID");
if (parsed.application_url !== appUrl.origin)
  throw new Error("rendered application_url does not match SHOPIFY_APP_URL");
if (
  !Array.isArray(parsed.auth?.redirect_urls) ||
  parsed.auth.redirect_urls.length !== 1 ||
  parsed.auth.redirect_urls[0] !== `${appUrl.origin}/auth/callback`
)
  throw new Error("rendered redirect_urls do not match SHOPIFY_APP_URL");
if (parsed.app_proxy?.url !== appUrl.origin)
  throw new Error("rendered app-proxy URL does not match SHOPIFY_APP_URL");

await writeFile(output, config, { mode: 0o600 });
console.log(`Rendered ${output} for ${appUrl.origin}`);
