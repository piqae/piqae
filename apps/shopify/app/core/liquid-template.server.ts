import { Liquid } from "liquidjs";
import type { DocumentSpec } from "@piqae/sdk";

export interface LiquidDiagnostic {
  code: string;
  severity: "error" | "warning";
  message: string;
}
export interface LiquidResult {
  output: string;
  document: DocumentSpec;
  diagnostics: LiquidDiagnostic[];
}
const MAX_SOURCE = 64 * 1024;
const MAX_OUTPUT = 512 * 1024;
const forbidden = /{%-?\s*(include|render|layout)\b/i;
const engine = new Liquid({
  strictFilters: true,
  strictVariables: true,
  ownPropertyOnly: true,
  dynamicPartials: false,
  outputEscape: "escape",
  parseLimit: MAX_SOURCE,
  renderLimit: 100,
  memoryLimit: 2_000_000,
  cache: false,
});

export async function renderShopifyLiquid(
  source: string,
  input: Record<string, unknown>,
): Promise<LiquidResult> {
  if (Buffer.byteLength(source, "utf8") > MAX_SOURCE)
    throw new Error("LIQUID_SOURCE_LIMIT");
  if (forbidden.test(source))
    throw new Error("LIQUID_EXTERNAL_TEMPLATE_FORBIDDEN");
  const output = await engine.parseAndRender(source, structuredClone(input), {
    templateLimit: 10_000,
    renderLimit: 100,
    memoryLimit: 2_000_000,
  });
  if (Buffer.byteLength(output, "utf8") > MAX_OUTPUT)
    throw new Error("LIQUID_OUTPUT_LIMIT");
  return {
    output,
    document: {
      spec_version: "piqae.document/v1",
      page: { size: "a4", margin_mm: 10 },
      body: [{ type: "text", value: output }],
    },
    diagnostics: [
      {
        code: "SHOPIFY_LIQUID_SAFE_SUBSET",
        severity: "warning",
        message:
          "Includes, render/layout tags, theme objects, network, filesystem, and arbitrary Shopify filters are unavailable.",
      },
    ],
  };
}
