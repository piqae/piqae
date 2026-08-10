export const ADAPTER_API_VERSION = "piqae.adapter/v1" as const;
export const DOCUMENT_SPEC_VERSION = "piqae.document/v1" as const;

export type CompatibilityLevel = "exact" | "mapped" | "lossy" | "unsupported";

export interface CompatibilityFeature {
  readonly feature: string;
  readonly level: CompatibilityLevel;
  readonly notes: string;
}

export interface CompatibilityManifest {
  readonly manifestVersion: "piqae.adapter-compatibility/v1";
  readonly adapter: string;
  readonly adapterVersion: string;
  readonly sourceFormat: string;
  readonly targetFormat: typeof DOCUMENT_SPEC_VERSION;
  readonly reviewedSourceVersions: readonly string[];
  readonly execution: "conversion-only";
  readonly features: readonly CompatibilityFeature[];
}

export interface AdapterDiagnostic {
  readonly code: string;
  readonly severity: "warning" | "error";
  readonly path: string;
  readonly message: string;
  readonly feature?: string;
}

export interface PiqaeDocumentV1 {
  readonly spec_version: typeof DOCUMENT_SPEC_VERSION;
  readonly page: {
    readonly size: "a4" | "a5" | "letter" | "four-by-six" | "roll58mm" | "roll80mm";
    readonly margin_mm: number;
  };
  readonly body: readonly PdfmeAdapterOutputNodeV1[];
}

export type PiqaeTextValue = string | { readonly pointer: string };
/** Nodes currently emitted by the pdfme subset adapter, not the full document/v1 model. */
export type PdfmeAdapterOutputNodeV1 =
  | { readonly type: "text"; readonly value: PiqaeTextValue; readonly font_size: number }
  | { readonly type: "qr"; readonly value: PiqaeTextValue; readonly size_mm: number }
  | { readonly type: "spacer"; readonly height_mm: number }
  | { readonly type: "page_break" };

export interface ConversionResult {
  readonly adapterApiVersion: typeof ADAPTER_API_VERSION;
  readonly adapter: string;
  readonly adapterVersion: string;
  readonly sourceFormat: string;
  readonly targetFormat: typeof DOCUMENT_SPEC_VERSION;
  readonly fidelity: "exact" | "lossy" | "incompatible";
  readonly document?: PiqaeDocumentV1;
  readonly warnings: readonly AdapterDiagnostic[];
  readonly errors: readonly AdapterDiagnostic[];
}

export interface ConvertOptions {
  /** Reject any conversion that would knowingly change layout or semantics. Default: true. */
  readonly strict?: boolean;
}

export interface DocumentAdapter<TSource = unknown> {
  readonly apiVersion: typeof ADAPTER_API_VERSION;
  readonly id: string;
  readonly version: string;
  readonly compatibility: CompatibilityManifest;
  convert(source: TSource, options?: ConvertOptions): ConversionResult;
}
