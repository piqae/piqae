import {
  canonicalToLiquid,
  liquidToCanonical,
} from "./liquid-document-adapter";
import type { PrintPacket } from "./template-model";

export type OrderPrinterImportFidelity =
  | "exact"
  | "mapped"
  | "lossy"
  | "unsupported";

export type OrderPrinterImportDiagnostic = {
  fidelity: OrderPrinterImportFidelity;
  code: string;
  message: string;
};

export type OrderPrinterImportResult =
  | {
      ok: true;
      document: PrintPacket;
      normalizedLiquid: string;
      originalSource: string;
      diagnostics: OrderPrinterImportDiagnostic[];
    }
  | { ok: false; diagnostics: OrderPrinterImportDiagnostic[] };

// Leaves room for the canonical AST and diagnostics in the 256 KiB envelope.
const MAX_SOURCE_BYTES = 64 * 1024;
const FORBIDDEN = [
  /<\s*(script|iframe|object|embed|link|meta|base)\b/i,
  /\son[a-z]+\s*=/i,
  /(?:url\s*\(|@import|javascript:|data:text\/html)/i,
  /{%[-]?\s*(include|render|section|layout)\b/i,
];

/**
 * Imports the common Forsberg/Order Printer Pro HTML + Liquid profile without
 * evaluating it. This is a syntax mapper, not a Liquid or browser runtime.
 */
export function importOrderPrinterProTemplate(
  source: string,
): OrderPrinterImportResult {
  if (!source.trim() || Buffer.byteLength(source, "utf8") > MAX_SOURCE_BYTES)
    return rejected(
      "source_size",
      "Template must be between 1 byte and 64 KiB",
    );
  if (FORBIDDEN.some((pattern) => pattern.test(source)))
    return rejected(
      "unsafe_construct",
      "Scripts, network resources, includes, renders and event handlers cannot be imported",
    );

  const diagnostics: OrderPrinterImportDiagnostic[] = [];
  let liquid = source.replace(/<!--[\s\S]*?-->/g, "");
  liquid = collectAssignments(liquid, diagnostics);
  liquid = mapLegacyPaths(liquid, diagnostics);
  liquid = mapMarkers(liquid, diagnostics);
  const theme = parseSafeTheme(liquid, diagnostics);
  liquid = liquid.replace(/<style\b[^>]*>[\s\S]*?<\/style\s*>/gi, "");
  liquid = mapHtmlStructure(liquid, diagnostics);
  liquid = normalizeFilters(liquid, diagnostics);
  liquid = `{% for order in orders limit: 250 %}\n${liquid}\n{% endfor %}`;

  const conversion = liquidToCanonical(liquid);
  if (!conversion.ok)
    return {
      ok: false,
      diagnostics: [
        ...diagnostics,
        ...conversion.diagnostics.map((item) => ({
          fidelity: "unsupported" as const,
          code: item.code,
          message: `Line ${item.line}:${item.column}: ${item.message}`,
        })),
      ],
    };
  conversion.document.theme = { ...conversion.document.theme, ...theme };
  diagnostics.unshift({
    fidelity: "exact",
    code: "bounded_import",
    message:
      "Imported without executing Liquid, HTML, CSS, plugins or network requests",
  });
  return {
    ok: true,
    document: conversion.document,
    normalizedLiquid: canonicalToLiquid(conversion.document).source,
    originalSource: source,
    diagnostics,
  };
}

function collectAssignments(
  source: string,
  diagnostics: OrderPrinterImportDiagnostic[],
) {
  const constants = new Map<string, string>();
  let output = source.replace(
    /{%[-]?\s*assign\s+([a-z_]\w*)\s*=\s*("[^"]{0,500}"|'[^']{0,500}'|true|false|-?\d+(?:\.\d+)?)\s*[-]?%}/gi,
    (_match, name: string, value: string) => {
      constants.set(name, /^['"]/.test(value) ? value.slice(1, -1) : value);
      diagnostics.push({
        fidelity: "mapped",
        code: "assign_constant",
        message: `Mapped constant assignment '${name}'`,
      });
      return "";
    },
  );
  for (const [name, value] of constants)
    output = output.replace(
      new RegExp(`{{\\s*${escapeRegex(name)}\\s*}}`, "g"),
      value,
    );
  if (/{%[-]?\s*assign\b/i.test(output))
    diagnostics.push({
      fidelity: "unsupported",
      code: "dynamic_assign",
      message: "Dynamic assign expressions were not imported",
    });
  output = output.replace(/{%[-]?\s*assign\b[\s\S]*?[-]?%}/gi, "");
  return output;
}

function mapLegacyPaths(
  source: string,
  diagnostics: OrderPrinterImportDiagnostic[],
) {
  const mappings: Array<[RegExp, string]> = [
    [/\border\.line_items\b/g, "order.lineItems"],
    [/\border\.shipping_address\b/g, "order.shippingAddress"],
    [/\border\.billing_address\b/g, "order.shippingAddress"],
    [/(?<!\.)\bline_items\b/g, "order.lineItems"],
    [/(?<!\.)\bshipping_address\b/g, "order.shippingAddress"],
    [/(?<!\.)\bbilling_address\b/g, "order.shippingAddress"],
    [/\border\.created_at\b/g, "order.createdAt"],
    [/\border\.total_price\b/g, "order.total"],
    [/\border\.subtotal_price\b/g, "order.subtotal"],
    [/\border\.tax_price\b/g, "order.tax"],
    [/\bline_item\.variant_title\b/g, "line_item.variant.title"],
    [/\bline_item\.product_title\b/g, "line_item.product.title"],
    [/\bline_item\.line_price\b/g, "line_item.total"],
    [/\bline_item\.original_price\b/g, "line_item.unitPrice"],
    [/\bshop\.name\b/g, "shop"],
  ];
  let output = source;
  let mapped = false;
  for (const [pattern, replacement] of mappings) {
    if (pattern.test(output)) mapped = true;
    pattern.lastIndex = 0;
    output = output.replace(pattern, replacement);
  }
  if (mapped)
    diagnostics.push({
      fidelity: "mapped",
      code: "shopify_fields",
      message: "Mapped common Shopify order, address and line-item fields",
    });
  if (/order\.shippingAddress/.test(output) && /billing_address/i.test(source))
    diagnostics.push({
      fidelity: "lossy",
      code: "billing_address_fallback",
      message:
        "Billing address uses the shipping address until billing data is enabled",
    });
  return output;
}

function mapMarkers(
  source: string,
  diagnostics: OrderPrinterImportDiagnostic[],
) {
  let output = source;
  output = output.replace(
    /<[^>]*(?:class=["'][^"']*\bqr(?:code)?\b[^"']*["']|data-piqae=["']qr["'])[^>]*>[\s\S]*?<\/[^>]+>/gi,
    "{% piqae_qr order.statusUrl %}",
  );
  output = output.replace(
    /<[^>]*(?:class=["'][^"']*\bbarcode\b[^"']*["']|data-piqae=["']barcode["'])[^>]*>[\s\S]*?<\/[^>]+>/gi,
    "{% piqae_barcode order.name symbology: code128 %}",
  );
  if (/piqae_(qr|barcode)/.test(output))
    diagnostics.push({
      fidelity: "mapped",
      code: "machine_readable_marker",
      message: "Mapped QR and barcode markers to native document primitives",
    });
  return output;
}

function mapHtmlStructure(
  source: string,
  diagnostics: OrderPrinterImportDiagnostic[],
) {
  let output = source
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<hr\b[^>]*\/?>/gi, "\n{% piqae_divider %}\n")
    .replace(
      /<\/(p|div|section|header|footer|address|h[1-6]|li|tr|table)>/gi,
      "\n\n",
    )
    .replace(
      /<(p|div|section|header|footer|address|h[1-6]|ul|ol|li|table|thead|tbody|tr|td|th)\b[^>]*>/gi,
      "",
    )
    .replace(/<img\b[^>]*>/gi, () => {
      diagnostics.push({
        fidelity: "unsupported",
        code: "image_requires_asset",
        message:
          "Images require an uploaded Shopify CDN asset and were omitted",
      });
      return "";
    });
  if (/<[^>]+>/.test(output)) {
    diagnostics.push({
      fidelity: "lossy",
      code: "html_decoration",
      message: "Decorative HTML was flattened into reflowing document content",
    });
    output = output.replace(/<[^>]+>/g, "");
  }
  if (
    /(display\s*:\s*(flex|grid)|class=["'][^"']*(header|address|footer))/i.test(
      source,
    )
  )
    diagnostics.push({
      fidelity: "lossy",
      code: "layout_reflow",
      message:
        "Header, address, flex and grid regions were imported as safe reflowing blocks",
    });
  return decodeEntities(output);
}

function normalizeFilters(
  source: string,
  diagnostics: OrderPrinterImportDiagnostic[],
) {
  let output = source.replace(/\|\s*(escape|strip_html|strip)\b/gi, "");
  if (/\|\s*(escape|strip_html|strip)\b/i.test(source))
    diagnostics.push({
      fidelity: "exact",
      code: "safe_filter",
      message: "Removed output-only escaping filters after safe parsing",
    });
  output = output.replace(/\|\s*money(?:_with_currency)?\b/gi, "| money");
  output = output.replace(/\|\s*date\s*:\s*[^}|]+/gi, "| date");
  output = output.replace(/\|\s*(default|newline_to_br)\s*:\s*[^}|]+/gi, "");
  if (/\|\s*(default|newline_to_br)\b/i.test(source))
    diagnostics.push({
      fidelity: "lossy",
      code: "presentation_filter",
      message: "Default/newline presentation filters were simplified",
    });
  return output;
}

function parseSafeTheme(
  source: string,
  diagnostics: OrderPrinterImportDiagnostic[],
): Partial<PrintPacket["theme"]> {
  const css = [...source.matchAll(/<style\b[^>]*>([\s\S]*?)<\/style\s*>/gi)]
    .map((match) => match[1])
    .join("\n");
  const body =
    /(?:body|\.document|\.template)\s*\{([^}]*)}/i.exec(css)?.[1] ?? "";
  const theme: Partial<PrintPacket["theme"]> = {};
  const font = /font-size\s*:\s*(\d+(?:\.\d+)?)\s*(pt|px)/i.exec(body);
  if (font)
    theme.font_size_pt = Math.min(
      24,
      Math.max(
        6,
        Number(font[1]) * (font[2]!.toLowerCase() === "px" ? 0.75 : 1),
      ),
    );
  const line = /line-height\s*:\s*(\d+(?:\.\d+)?)/i.exec(body);
  if (line) theme.line_height = Math.min(2.5, Math.max(0.8, Number(line[1])));
  const color = /color\s*:\s*#([0-9a-f]{6})\b/i.exec(body)?.[1];
  if (color)
    theme.text_color = {
      red: parseInt(color.slice(0, 2), 16),
      green: parseInt(color.slice(2, 4), 16),
      blue: parseInt(color.slice(4, 6), 16),
    };
  if (Object.keys(theme).length)
    diagnostics.push({
      fidelity: "mapped",
      code: "document_theme",
      message: "Mapped safe document font size, line height and text color",
    });
  if (
    css &&
    /(?:position|float|transform|background-image|@font-face)\s*:/i.test(css)
  )
    diagnostics.push({
      fidelity: "unsupported",
      code: "unsafe_css",
      message:
        "Positioning, transforms, background images and embedded fonts were omitted",
    });
  return theme;
}

function decodeEntities(value: string) {
  return value
    .replace(/&nbsp;/gi, " ")
    .replace(/&amp;/gi, "&")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&quot;/gi, '"')
    .replace(/&#39;/gi, "'");
}

function escapeRegex(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function rejected(code: string, message: string): OrderPrinterImportResult {
  return {
    ok: false,
    diagnostics: [{ fidelity: "unsupported", code, message }],
  };
}
