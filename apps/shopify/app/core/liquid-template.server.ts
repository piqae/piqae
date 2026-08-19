import { liquidToCanonical } from "./liquid-document-adapter";
import type { BusinessDocument } from "./template-model";
export interface LiquidResult {
  output: string;
  document: BusinessDocument;
  diagnostics: { code: string; severity: "warning"; message: string }[];
}
/** Compatibility helper for text-only callers. Publishing uses the structural compiler directly. */
export async function renderShopifyLiquid(
  source: string,
  input: Record<string, unknown>,
): Promise<LiquidResult> {
  if (new TextEncoder().encode(source).byteLength > 64 * 1024)
    throw new Error("LIQUID_SOURCE_LIMIT");
  if (/{%-?\s*(include|render|layout)\b/i.test(source))
    throw new Error("LIQUID_EXTERNAL_TEMPLATE_FORBIDDEN");
  const compiled = liquidToCanonical(source);
  if (!compiled.ok)
    throw new Error(`LIQUID_${compiled.diagnostics[0]!.code.toUpperCase()}`);
  const output = source.replace(
    /{{\s*([\w.]+)(?:\s*\|[^}]*)?\s*}}/g,
    (_all, name: string) =>
      escapeHtml(String(readPath(input, name.split(".")) ?? "")),
  );
  return {
    output,
    document: compiled.document,
    diagnostics: [
      {
        code: "SHOPIFY_LIQUID_BUSINESS_DOCUMENT_PROFILE",
        severity: "warning",
        message:
          "Only bounded data, formatting, conditions and iteration are supported.",
      },
    ],
  };
}
function readPath(input: unknown, path: string[]): unknown {
  let value = input;
  for (const key of path) {
    if (!value || typeof value !== "object" || !Object.hasOwn(value, key))
      return undefined;
    value = (value as Record<string, unknown>)[key];
  }
  return value;
}
function escapeHtml(value: string) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}
