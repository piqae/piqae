import {
  ADAPTER_API_VERSION,
  DOCUMENT_SPEC_VERSION,
  type AdapterDiagnostic,
  type CompatibilityManifest,
  type ConversionResult,
  type ConvertOptions,
  type DocumentAdapter,
  type PiqaeDocumentV1,
  type PdfmeAdapterOutputNodeV1,
  type PiqaeTextValue,
} from "./types.js";

type JsonObject = Record<string, unknown>;

export const pdfmeCompatibility: CompatibilityManifest = {
  manifestVersion: "piqae.adapter-compatibility/v1",
  adapter: "pdfme",
  adapterVersion: "1.0.0",
  sourceFormat: "pdfme.template",
  targetFormat: DOCUMENT_SPEC_VERSION,
  reviewedSourceVersions: ["5.x", "6.x"],
  execution: "conversion-only",
  features: [
    { feature: "blank-page.a4", level: "exact", notes: "Object basePdf dimensions 210 x 297 mm." },
    { feature: "blank-page.a5", level: "exact", notes: "Object basePdf dimensions 148 x 210 mm." },
    { feature: "blank-page.letter", level: "exact", notes: "Object basePdf dimensions 215.9 x 279.4 mm." },
    { feature: "blank-page.4x6", level: "exact", notes: "Object basePdf dimensions 101.6 x 152.4 mm." },
    { feature: "blank-page.roll58mm", level: "exact", notes: "Object basePdf width 58 mm and finite height from 1 through 2000 mm." },
    { feature: "blank-page.roll80mm", level: "exact", notes: "Object basePdf width 80 mm and finite height from 1 through 2000 mm." },
    { feature: "multipage", level: "mapped", notes: "Each schemas entry becomes a page separated by page_break." },
    { feature: "text.content", level: "mapped", notes: "Named schemas bind using RFC 6901 JSON Pointer escaping (~ as ~0 and / as ~1); unnamed content is literal." },
    { feature: "text.font-size", level: "mapped", notes: "Font size is retained; Piqae controls font face and shaping." },
    { feature: "qrcode", level: "mapped", notes: "Named QR schemas bind using an RFC 6901 JSON Pointer token (~ becomes ~0 and / becomes ~1)." },
    { feature: "absolute-positioning", level: "lossy", notes: "Vertical spacing is approximated; horizontal coordinates and boxes are not retained in document/v1." },
    { feature: "base-pdf", level: "unsupported", notes: "Existing/background PDF data is never fetched or executed." },
    { feature: "images", level: "unsupported", notes: "document/v1 has no image node." },
    { feature: "tables", level: "unsupported", notes: "document/v1 has a bounded flow-table node, but this adapter does not yet map pdfme table schemas." },
    { feature: "barcodes", level: "unsupported", notes: "Only QR is available in document/v1." },
    { feature: "custom-fonts", level: "unsupported", notes: "No remote or embedded source fonts are accepted by this adapter." },
    { feature: "plugins", level: "unsupported", notes: "Arbitrary pdfme plugins or JavaScript are never loaded or run." },
    { feature: "remote-assets", level: "unsupported", notes: "Network URLs are never fetched." }
  ]
};

function object(value: unknown): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function close(a: number, b: number): boolean { return Math.abs(a - b) <= 0.6; }

function pageSize(base: JsonObject): PiqaeDocumentV1["page"]["size"] | undefined {
  const width = base.width;
  const height = base.height;
  if (typeof width !== "number" || typeof height !== "number" || !Number.isFinite(width) || !Number.isFinite(height)) return undefined;
  if (close(width, 210) && close(height, 297)) return "a4";
  if (close(width, 148) && close(height, 210)) return "a5";
  if (close(width, 215.9) && close(height, 279.4)) return "letter";
  if (close(width, 101.6) && close(height, 152.4)) return "four-by-six";
  if (close(width, 58) && height >= 1 && height <= 2000) return "roll58mm";
  if (close(width, 80) && height >= 1 && height <= 2000) return "roll80mm";
  return undefined;
}

function pointer(name: string): string {
  return `/${name.replaceAll("~", "~0").replaceAll("/", "~1")}`;
}

function diagnostic(code: string, severity: "warning" | "error", path: string, message: string, feature?: string): AdapterDiagnostic {
  return feature === undefined ? { code, severity, path, message } : { code, severity, path, message, feature };
}

function valueFor(schema: JsonObject): PiqaeTextValue | undefined {
  if (typeof schema.name === "string" && schema.name.length > 0) return { pointer: pointer(schema.name) };
  return typeof schema.content === "string" ? schema.content : undefined;
}

/** Conversion only: this adapter never imports or executes pdfme or its plugins. */
export const pdfmeAdapter: DocumentAdapter<unknown> = {
  apiVersion: ADAPTER_API_VERSION,
  id: "pdfme",
  version: "1.0.0",
  compatibility: pdfmeCompatibility,
  convert(source: unknown, options: ConvertOptions = {}): ConversionResult {
    const strict = options.strict ?? true;
    const warnings: AdapterDiagnostic[] = [];
    const errors: AdapterDiagnostic[] = [];
    const body: PdfmeAdapterOutputNodeV1[] = [];
    if (!object(source)) errors.push(diagnostic("PDFME_INVALID_TEMPLATE", "error", "$", "Template must be a JSON object."));
    const template = object(source) ? source : {};
    if (!object(template.basePdf)) {
      errors.push(diagnostic("PDFME_BASE_PDF_UNSUPPORTED", "error", "$.basePdf", "Only a blank object basePdf is convertible; use pdfme locally and submit PDF bytes for background PDFs.", "base-pdf"));
    }
    const base = object(template.basePdf) ? template.basePdf : {};
    const size = pageSize(base);
    if (size === undefined) errors.push(diagnostic("PDFME_PAGE_SIZE_UNSUPPORTED", "error", "$.basePdf", "Blank page dimensions do not match a Piqae document/v1 page preset.", "blank-page"));
    if (!Array.isArray(template.schemas)) errors.push(diagnostic("PDFME_SCHEMAS_REQUIRED", "error", "$.schemas", "schemas must be an array of page arrays."));
    const pages = Array.isArray(template.schemas) ? template.schemas : [];
    pages.forEach((rawPage, pageIndex) => {
      if (!Array.isArray(rawPage)) {
        errors.push(diagnostic("PDFME_PAGE_INVALID", "error", `$.schemas[${pageIndex}]`, "Page schemas must be an array."));
        return;
      }
      if (pageIndex > 0) body.push({ type: "page_break" });
      const schemas = rawPage.flatMap((value, sourceIndex) => object(value) ? [{ schema: value, sourceIndex }] : []).sort((left, right) => {
        const ly = object(left.schema.position) && typeof left.schema.position.y === "number" ? left.schema.position.y : 0;
        const ry = object(right.schema.position) && typeof right.schema.position.y === "number" ? right.schema.position.y : 0;
        return ly - ry;
      });
      let cursorY = 0;
      schemas.forEach(({ schema, sourceIndex }) => {
        const path = `$.schemas[${pageIndex}][${sourceIndex}]`;
        const type = schema.type;
        const position = object(schema.position) ? schema.position : {};
        const y = typeof position.y === "number" && Number.isFinite(position.y) ? position.y : cursorY;
        if (y > cursorY) body.push({ type: "spacer", height_mm: y - cursorY });
        const hasAbsoluteLayout = object(schema.position)
          || Object.hasOwn(schema, "width")
          || Object.hasOwn(schema, "height");
        if (hasAbsoluteLayout) {
          const layoutMessage = "pdfme absolute boxes are reduced to vertical flow; horizontal position, width, height and overlap are not retained.";
          (strict ? errors : warnings).push(diagnostic("PDFME_LAYOUT_LOSSY", strict ? "error" : "warning", path, layoutMessage, "absolute-positioning"));
        }
        const value = valueFor(schema);
        if (value === undefined) {
          errors.push(diagnostic("PDFME_VALUE_REQUIRED", "error", path, "Schema requires a non-empty name or literal string content."));
          return;
        }
        if (type === "text") {
          const fontSize = typeof schema.fontSize === "number" && Number.isFinite(schema.fontSize) ? schema.fontSize : 10;
          body.push({ type: "text", value, font_size: fontSize });
        } else if (type === "qrcode") {
          const width = typeof schema.width === "number" && Number.isFinite(schema.width) ? schema.width : 24;
          body.push({ type: "qr", value, size_mm: width });
        } else {
          const typeLabel = typeof type === "string" ? JSON.stringify(type) : "unknown";
          errors.push(diagnostic("PDFME_SCHEMA_TYPE_UNSUPPORTED", "error", `${path}.type`, `Schema type ${typeLabel} is unsupported because hosted conversion never runs pdfme plugins.`, "plugins"));
        }
        const height = typeof schema.height === "number" && Number.isFinite(schema.height) ? schema.height : 0;
        cursorY = Math.max(cursorY, y + height);
      });
    });
    const fidelity = errors.length > 0 ? "incompatible" : warnings.length > 0 ? "lossy" : "exact";
    const document = errors.length === 0 && size !== undefined
      ? { spec_version: DOCUMENT_SPEC_VERSION, page: { size, margin_mm: 0 }, body } satisfies PiqaeDocumentV1
      : undefined;
    return {
      adapterApiVersion: ADAPTER_API_VERSION,
      adapter: "pdfme",
      adapterVersion: "1.0.0",
      sourceFormat: "pdfme.template",
      targetFormat: DOCUMENT_SPEC_VERSION,
      fidelity,
      ...(document === undefined ? {} : { document }),
      warnings,
      errors,
    };
  }
};
