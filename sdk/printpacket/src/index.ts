export const PRINTPACKET_V1 = "printpacket/v1" as const;
export const LEGACY_PIQAE_DOCUMENT_V1 = "piqae.business-document/v1" as const;
export const PDF_BASE14_V1 = "printpacket.pdf-base14/v1" as const;
export const CONFORMANCE_CORE_V1 = "printpacket.conformance/core-v1" as const;

export type JsonPrimitive = string | number | boolean | null;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };
export type JsonObject = { [key: string]: JsonValue };

export type Expression =
  | { type: "literal"; value: JsonValue }
  | { type: "path" | "current_path"; path: string[] }
  | { type: "coalesce" | "concat"; values: Expression[] }
  | { type: "compare"; operator: "equal" | "not_equal" | "less" | "less_or_equal" | "greater" | "greater_or_equal"; left: Expression; right: Expression }
  | { type: "boolean"; operator: "and" | "or"; values: Expression[] }
  | { type: "not" | "exists"; value: Expression }
  | { type: "contains"; collection: Expression; value: Expression }
  | { type: "page_number" | "page_count" }
  | { type: "arithmetic"; operator: "add" | "subtract" | "multiply" | "divide"; left: Expression; right: Expression }
  | { type: "format_number"; value: Expression; decimals?: number }
  | { type: "format_money"; amount: Expression; currency: Expression; decimals?: number }
  | { type: "format_date"; value: Expression; format: "iso_date" | "day_month_year" | "month_day_year" }
  | { type: "format_string"; value: Expression; operation: "trim" | "uppercase_ascii" | "lowercase_ascii" };

export type TextStyle = {
  bold?: boolean; italic?: boolean; underline?: boolean; font_size_pt?: number;
  align?: "left" | "center" | "right";
  color?: { red: number; green: number; blue: number };
};
export type Inline =
  | { type: "text"; value: string; style?: TextStyle }
  | { type: "value"; value: Expression; style?: TextStyle }
  | { type: "line_break" };

type ChildrenNode = { type: "section" | "stack" | "row"; children: Node[]; gap_mm?: number };
export type Node =
  | ChildrenNode
  | { type: "box"; children: Node[]; style?: JsonObject }
  | { type: "paragraph" | "heading"; content: Inline[]; level?: number; style?: TextStyle }
  | { type: "grid"; columns: number[]; children: Node[]; gap_mm?: number }
  | { type: "table"; items: Expression; columns: Array<{ header: Inline[]; cell: Inline[]; width?: number; align?: "left" | "center" | "right" }>; repeat_header?: boolean; empty?: Node[]; style?: JsonObject }
  | { type: "repeat"; items: Expression; children: Node[]; gap_mm?: number }
  | { type: "data_list"; items: Expression; header?: Node[]; item: Node[]; empty?: Node[]; repeat_header?: boolean; gap_mm?: number }
  | { type: "conditional"; condition: Expression; then: Node[]; else?: Node[] }
  | { type: "spacer"; height_mm: number }
  | { type: "divider"; width_pt?: number }
  | { type: "page_break" }
  | { type: "keep_together"; children: Node[] }
  | { type: "image"; resource: string; width_mm: number; height_mm: number; fit?: "contain" | "fill" | "scale_down" }
  | { type: "image_value"; resource: Expression; width_mm: number; height_mm: number; fit?: "contain" | "fill" | "scale_down" }
  | { type: "qr"; value: Expression; size_mm: number; error_correction?: "L" | "M" | "Q" | "H" }
  | { type: "barcode"; value: Expression; symbology: "code128"; width_mm: number; height_mm: number; human_readable?: boolean };

export type Media =
  | { kind: "paged"; size: "a4" | "a5" | "letter"; orientation?: "portrait" | "landscape"; margins?: Edges }
  | { kind: "continuous"; width_mm: number; margins?: Edges }
  | { kind: "label"; width_mm: number; height_mm: number; margins?: Edges };
export type Edges = { top_mm: number; right_mm: number; bottom_mm: number; left_mm: number };
export type PrintPacketV1 = {
  format: typeof PRINTPACKET_V1 | typeof LEGACY_PIQAE_DOCUMENT_V1;
  media: Media;
  theme?: { font_size_pt?: number; line_height?: number; text_color?: { red: number; green: number; blue: number } };
  resources?: Record<string, { type: "image"; digest: `sha256:${string}`; media_type: "image/jpeg"; byte_length: number }>;
  header?: { first?: Node[]; default?: Node[] };
  body: Node[];
  footer?: { default?: Node[]; last?: Node[] };
};

export type Feature =
  | "media_paged" | "media_continuous" | "media_label" | "layout_flow"
  | "layout_grid" | "layout_table" | "layout_regions" | "layout_keep_together"
  | "data_expressions" | "data_repeat" | "image_jpeg" | "barcode_qr"
  | "barcode_code128" | "typography_base14_windows1252";
export type OutputTarget =
  | { kind: "pdf"; profile: string }
  | { kind: "printer_native"; language: string; profile: string; dpi: number; printable_width_dots: number };
export type TemplateManifest = {
  standard: "PrintPacket"; specification_version: string; canonical_json: string;
  canonical_sha256: string; canonical_bytes: number; required_features: Feature[];
  resource_count: number; resource_bytes: number;
};

export function definePacket<const T extends PrintPacketV1>(packet: T): T { return packet; }
export function defineData<const T extends JsonObject>(data: T): T { return data; }

export function normalizeFormat(format: PrintPacketV1["format"]): typeof PRINTPACKET_V1 {
  if (format !== PRINTPACKET_V1 && format !== LEGACY_PIQAE_DOCUMENT_V1) throw new Error("Unsupported PrintPacket format");
  return PRINTPACKET_V1;
}

export function preflightPacket(packet: PrintPacketV1): void {
  normalizeFormat(packet.format);
  if (!Array.isArray(packet.body)) throw new Error("PrintPacket body must be an array");
  const encoded = new TextEncoder().encode(JSON.stringify(packet));
  if (encoded.byteLength > 1024 * 1024) throw new Error("PrintPacket template exceeds 1 MiB");
  let count = 0;
  const walk = (nodes: Node[], depth: number): void => {
    if (depth > 32) throw new Error("PrintPacket nesting exceeds 32 levels");
    for (const node of nodes) {
      count += 1;
      if (count > 20_000) throw new Error("PrintPacket exceeds 20,000 nodes");
      if ("children" in node) walk(node.children, depth + 1);
      if (node.type === "conditional") { walk(node.then, depth + 1); walk(node.else ?? [], depth + 1); }
      if (node.type === "data_list") { walk(node.header ?? [], depth + 1); walk(node.item, depth + 1); walk(node.empty ?? [], depth + 1); }
      if (node.type === "table") walk(node.empty ?? [], depth + 1);
    }
  };
  walk([...(packet.header?.first ?? []), ...(packet.header?.default ?? []), ...packet.body, ...(packet.footer?.default ?? []), ...(packet.footer?.last ?? [])], 0);
}

export function requiredFeatures(packet: PrintPacketV1): Feature[] {
  preflightPacket(packet);
  const features = new Set<Feature>(["layout_flow", "data_expressions", "typography_base14_windows1252"]);
  features.add(packet.media.kind === "paged" ? "media_paged" : packet.media.kind === "continuous" ? "media_continuous" : "media_label");
  if (packet.header || packet.footer) features.add("layout_regions");
  if (Object.keys(packet.resources ?? {}).length > 0) features.add("image_jpeg");
  const walk = (nodes: Node[]): void => {
    for (const node of nodes) {
      if (node.type === "grid") features.add("layout_grid");
      if (node.type === "table") features.add("layout_table");
      if (node.type === "repeat" || node.type === "data_list" || node.type === "table") features.add("data_repeat");
      if (node.type === "keep_together") features.add("layout_keep_together");
      if (node.type === "image" || node.type === "image_value") features.add("image_jpeg");
      if (node.type === "qr") features.add("barcode_qr");
      if (node.type === "barcode") features.add("barcode_code128");
      if ("children" in node) walk(node.children);
      if (node.type === "conditional") { walk(node.then); walk(node.else ?? []); }
      if (node.type === "data_list") { walk(node.header ?? []); walk(node.item); walk(node.empty ?? []); }
      if (node.type === "table") walk(node.empty ?? []);
    }
  };
  walk(packet.body);
  return [...features].sort();
}

function sortedJson(value: JsonValue): JsonValue {
  if (Array.isArray(value)) return value.map(sortedJson);
  if (value !== null && typeof value === "object") return Object.fromEntries(Object.keys(value).sort().map((key) => [key, sortedJson(value[key] as JsonValue)]));
  return value;
}
export function canonicalData(data: JsonObject): string { return JSON.stringify(sortedJson(data)); }
function hex(bytes: ArrayBuffer): string { return [...new Uint8Array(bytes)].map((byte) => byte.toString(16).padStart(2, "0")).join(""); }
function canonicalTarget(target: OutputTarget): string {
  return target.kind === "pdf"
    ? JSON.stringify({ kind: target.kind, profile: target.profile })
    : JSON.stringify({ kind: target.kind, language: target.language, profile: target.profile, dpi: target.dpi, printable_width_dots: target.printable_width_dots });
}
export async function renderCacheKey(manifest: TemplateManifest, data: JsonObject, target: OutputTarget = { kind: "pdf", profile: PDF_BASE14_V1 }): Promise<string> {
  const input = `printpacket.render-cache/v1\0${manifest.canonical_sha256}\0${CONFORMANCE_CORE_V1}\0${canonicalTarget(target)}\0${canonicalData(data)}`;
  return hex(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(input)));
}
