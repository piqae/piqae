import type { DocumentPointer, DocumentSpec } from "@piqae/sdk";

export type TemplateEditorMode = "visual" | "liquid" | "native";
export type ExternalAsset = {
  url: string;
  sha256: string;
  contentType: "font/woff2" | "image/png" | "image/jpeg" | "image/svg+xml";
  bytes: number;
};
export type TemplateEnvelope = {
  schema: "piqae.shopify-template/v1";
  canonical: DocumentSpec;
  editor: {
    mode: TemplateEditorMode;
    pdfme?: PdfmeVisualModel;
    liquid?: string;
    roundTrip: "lossless" | "lossy" | "unsupported";
    warnings: string[];
  };
  assets: ExternalAsset[];
  system?: { key: string; immutable: true };
  published?: {
    piqaeTemplateId: string;
    piqaeRevisionId: string;
    canonicalDigest: string;
  };
};
export type PdfmeVisualField = {
  id: string;
  type: "text" | "image" | "qrcode" | "line";
  x: number;
  y: number;
  width: number;
  height: number;
  binding?: string;
  text?: string;
  fontSize?: number;
};
export type PdfmeTemplateSubset = {
  basePdf: {
    width: number;
    height: number;
    padding: [number, number, number, number];
  };
  schemas: Array<
    Array<{
      name: string;
      type: string;
      content?: string;
      position: { x: number; y: number };
      width: number;
      height: number;
      fontSize?: number;
      rotate?: number;
      opacity?: number;
      /** Piqae round-trip metadata; null means literal content. */
      piqaeBinding?: string | null;
    }>
  >;
};
export type PdfmeVisualModel = {
  schema: "pdfme-compatible/v1";
  page: "A4" | "A5" | "Letter" | "80mm";
  fields: PdfmeVisualField[];
  /** Native pdfme data-only template. Plugins/assets are never persisted here. */
  template?: PdfmeTemplateSubset;
};

const ALLOWED_ASSET_HOSTS = new Set(
  (process.env.TEMPLATE_ASSET_CDN_HOSTS ?? "cdn.shopify.com")
    .split(",")
    .map((value) => value.trim().toLowerCase())
    .filter(Boolean),
);
export const ASSET_LIMITS = {
  maxBytes: 2 * 1024 * 1024,
  maxAssets: 20,
  cacheSeconds: 3600,
} as const;

export function parseTemplateEnvelope(source: string): TemplateEnvelope {
  if (!source || new TextEncoder().encode(source).byteLength > 65_536)
    throw new Error("Document source is invalid");
  let parsed: unknown;
  try {
    parsed = JSON.parse(source);
  } catch {
    throw new Error("Document source must be valid JSON");
  }
  if (!parsed || typeof parsed !== "object")
    throw new Error("Document source must be an object");
  if (
    (parsed as { schema?: unknown }).schema === "piqae.document/v1" ||
    (parsed as { spec_version?: unknown }).spec_version === "piqae.document/v1"
  ) {
    return {
      schema: "piqae.shopify-template/v1",
      canonical: parsed as DocumentSpec,
      editor: { mode: "native", roundTrip: "unsupported", warnings: [] },
      assets: [],
    };
  }
  const envelope = parsed as TemplateEnvelope;
  if (
    envelope.schema !== "piqae.shopify-template/v1" ||
    envelope.canonical?.spec_version !== "piqae.document/v1" ||
    !["visual", "liquid", "native"].includes(envelope.editor?.mode)
  )
    throw new Error("Document source must use piqae.shopify-template/v1");
  validateAssets(envelope.assets ?? []);
  return envelope;
}

export function serializeTemplateEnvelope(value: TemplateEnvelope): string {
  validateAssets(value.assets);
  const source = JSON.stringify(value);
  if (new TextEncoder().encode(source).byteLength > 65_536)
    throw new Error("Document source exceeds 64 KiB");
  return source;
}

export function validateAssets(assets: ExternalAsset[]): void {
  if (assets.length > ASSET_LIMITS.maxAssets)
    throw new Error("Too many assets");
  for (const asset of assets) {
    let url: URL;
    try {
      url = new URL(asset.url);
    } catch {
      throw new Error("Asset URL is invalid");
    }
    if (url.protocol !== "https:" || !ALLOWED_ASSET_HOSTS.has(url.hostname))
      throw new Error("Asset must use an allowlisted HTTPS CDN");
    if (!/^[a-f0-9]{64}$/.test(asset.sha256))
      throw new Error("Published assets require a SHA-256 digest");
    if (
      !Number.isInteger(asset.bytes) ||
      asset.bytes < 1 ||
      asset.bytes > ASSET_LIMITS.maxBytes
    )
      throw new Error("Asset size is invalid");
    if (
      !(
        ["font/woff2", "image/png", "image/jpeg", "image/svg+xml"] as string[]
      ).includes(asset.contentType)
    )
      throw new Error("Asset content type is unsupported");
  }
}

export function visualCompatibility(model: PdfmeVisualModel): {
  roundTrip: TemplateEnvelope["editor"]["roundTrip"];
  warnings: string[];
} {
  const warnings: string[] = [];
  const fields = visualFields(model);
  if (fields.length > 200)
    warnings.push("Visual templates support at most 200 fields.");
  for (const field of fields) {
    if (
      !/^(\/|\.\/)[a-z0-9_./-]+$/i.test(field.binding ?? "/order/name") &&
      field.binding
    )
      warnings.push(`${field.id}: binding is not a supported JSON pointer.`);
  }
  for (const [pageIndex, page] of (model.template?.schemas ?? []).entries())
    for (const [index, schema] of page.entries()) {
      if (!["text", "qrcode", "line"].includes(schema.type))
        warnings.push(
          `Page ${pageIndex + 1}, item ${index + 1}: ${schema.type} is unsupported.`,
        );
      if ((schema.rotate ?? 0) !== 0 || (schema.opacity ?? 1) !== 1)
        warnings.push(
          `Page ${pageIndex + 1}, item ${index + 1}: rotation/opacity is lossy.`,
        );
    }
  const unsupported = warnings.some((warning) =>
    warning.includes("unsupported"),
  );
  return {
    roundTrip: unsupported
      ? "unsupported"
      : warnings.length
        ? "lossy"
        : "lossless",
    warnings,
  };
}

export function visualToCanonical(model: PdfmeVisualModel): DocumentSpec {
  const compatibility = visualCompatibility(model);
  if (compatibility.warnings.some((warning) => warning.includes("at most")))
    throw new Error("Visual model is outside the supported adapter subset");
  const size = model.page === "80mm" ? "roll80mm" : model.page.toLowerCase();
  if (compatibility.roundTrip !== "lossless")
    throw new Error(
      `Visual source is not exactly supported: ${compatibility.warnings.join(" ")}`,
    );
  const pages = model.template
    ? model.template.schemas.map((_, index) => visualPageFields(model, index))
    : [model.fields];
  const body: DocumentSpec["body"] = [];
  for (const [pageIndex, fields] of pages.entries()) {
    const children: Extract<
      DocumentSpec["body"][number],
      { type: "canvas" }
    >["children"] = [];
    if (pageIndex > 0) body.push({ type: "page_break" });
    for (const field of fields) {
      if (
        ![field.x, field.y, field.width, field.height].every(Number.isFinite) ||
        field.x < 0 ||
        field.y < 0 ||
        field.width <= 0 ||
        field.height <= 0
      )
        throw new Error(
          `${field.id}: canvas box must contain finite positive dimensions.`,
        );
      const box = {
        x_mm: field.x,
        y_mm: field.y,
        width_mm: field.width,
        height_mm: field.height,
      };
      if (field.type === "line") children.push({ type: "line", ...box });
      else if (field.type === "text")
        children.push({
          type: "text",
          value: field.binding
            ? { pointer: field.binding as DocumentPointer }
            : (field.text ?? ""),
          font_size: field.fontSize ?? 10,
          ...box,
        });
      else if (field.type === "qrcode")
        children.push({
          type: "qr",
          value: field.binding
            ? { pointer: field.binding as DocumentPointer }
            : (field.text ?? ""),
          ...box,
        });
      else
        throw new Error(
          "Images require a pinned asset and are not yet convertible by the hosted PDFme adapter",
        );
    }
    body.push({ type: "canvas", children });
  }
  return {
    spec_version: "piqae.document/v1",
    page: {
      size: size as DocumentSpec["page"]["size"],
      margin_mm: model.page === "80mm" ? 4 : 10,
    },
    body,
  };
}

export function visualFields(model: PdfmeVisualModel): PdfmeVisualField[] {
  if (!model.template) return model.fields;
  return model.template.schemas.flatMap((_, pageIndex) =>
    visualPageFields(model, pageIndex),
  );
}

function visualPageFields(
  model: PdfmeVisualModel,
  pageIndex: number,
): PdfmeVisualField[] {
  if (!model.template) return pageIndex === 0 ? model.fields : [];
  return (model.template.schemas[pageIndex] ?? []).map((schema, index) => {
    const hasBindingMetadata = Object.hasOwn(schema, "piqaeBinding");
    const binding = hasBindingMetadata
      ? (schema.piqaeBinding ?? undefined)
      : schema.name
        ? `/${schema.name.replaceAll("~", "~0").replaceAll("/", "~1")}`
        : undefined;
    return {
      id: `${pageIndex}-${index}-${schema.name}`,
      type: schema.type as PdfmeVisualField["type"],
      x: schema.position.x,
      y: schema.position.y,
      width: schema.width,
      height: schema.height,
      binding,
      text: binding ? undefined : schema.content,
      fontSize: schema.fontSize,
    };
  });
}

export function visualTemplate(model: PdfmeVisualModel): PdfmeTemplateSubset {
  if (model.template) return model.template;
  const sizes = {
    A4: [210, 297],
    A5: [148, 210],
    Letter: [215.9, 279.4],
    "80mm": [80, 297],
  } as const;
  const [width, height] = sizes[model.page];
  return {
    basePdf: { width, height, padding: [0, 0, 0, 0] },
    schemas: [
      model.fields.map((field) => ({
        name:
          field.binding
            ?.replace(/^\//, "")
            .replaceAll("~1", "/")
            .replaceAll("~0", "~") ?? field.id,
        type: field.type,
        content: field.text ?? "",
        position: { x: field.x, y: field.y },
        width: field.width,
        height: field.height,
        fontSize: field.fontSize,
        piqaeBinding: field.binding ?? null,
      })),
    ],
  };
}
