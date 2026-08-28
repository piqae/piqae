export const PRINTPACKET_V1 = "printpacket/v1" as const;
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
  format: typeof PRINTPACKET_V1;
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

export function normalizeFormat(format: string): typeof PRINTPACKET_V1 {
  if (format !== PRINTPACKET_V1) throw new Error("Unsupported PrintPacket format");
  return PRINTPACKET_V1;
}

export function preflightPacket(packet: PrintPacketV1): void {
  normalizeFormat(packet.format);
  if (!Array.isArray(packet.body)) throw new Error("PrintPacket body must be an array");
  if (Object.keys(packet.resources ?? {}).length > 100) throw new Error("PrintPacket exceeds 100 resources");
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
  walk([
    ...(packet.header?.first ?? []),
    ...(packet.header?.default ?? []),
    ...packet.body,
    ...(packet.footer?.default ?? []),
    ...(packet.footer?.last ?? [])
  ]);
  return [...features].sort();
}

const encoder = new TextEncoder();
const decoder = new TextDecoder();
const MAX_CANONICAL_DATA_BYTES = 4 * 1024 * 1024;
const MAX_CANONICAL_DATA_DEPTH = 128;

function scalarAt(value: string, index: number): readonly [number, number] {
    const first = value.charCodeAt(index);
    if (first >= 0xd800 && first <= 0xdbff) {
      const second = value.charCodeAt(index + 1);
      if (!(second >= 0xdc00 && second <= 0xdfff)) throw new Error("PrintPacket strings must contain valid Unicode scalar values");
      return [0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00), index + 2];
    }
    if (first >= 0xdc00 && first <= 0xdfff) throw new Error("PrintPacket strings must contain valid Unicode scalar values");
    return [first, index + 1];
}

function utf8Length(value: string): number {
  let length = 0;
  for (let index = 0; index < value.length;) {
    const [scalar, next] = scalarAt(value, index);
    length += scalar <= 0x7f ? 1 : scalar <= 0x7ff ? 2 : scalar <= 0xffff ? 3 : 4;
    index = next;
  }
  return length;
}

function compareUtf8(left: string, right: string): number {
  let leftIndex = 0;
  let rightIndex = 0;
  while (leftIndex < left.length && rightIndex < right.length) {
    const [leftScalar, leftNext] = scalarAt(left, leftIndex);
    const [rightScalar, rightNext] = scalarAt(right, rightIndex);
    const difference = leftScalar - rightScalar;
    if (difference !== 0) return difference;
    leftIndex = leftNext;
    rightIndex = rightNext;
  }
  return (left.length - leftIndex) - (right.length - rightIndex);
}

function canonicalNumber(value: number): string {
  if (!Number.isFinite(value)) throw new Error("PrintPacket data numbers must be finite");
  if (Number.isInteger(value) && Math.abs(value) > Number.MAX_SAFE_INTEGER) {
    throw new Error("PrintPacket integral data numbers must be JavaScript-safe integers");
  }
  const normalized = Object.is(value, -0) ? 0 : value;
  const buffer = new ArrayBuffer(8);
  new DataView(buffer).setFloat64(0, normalized, false);
  return [...new Uint8Array(buffer)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

/**
 * Produce the versioned, typed PrintPacket data encoding used in render cache
 * identities. It is deliberately not JSON text: binary64 number bits and
 * UTF-8 key ordering make the result identical in Rust and JavaScript.
 */
export function canonicalDataBytes(data: JsonObject): Uint8Array {
  const chunks: string[] = ["printpacket.canonical-data/v1\0"];
  let bytes = encoder.encode(chunks[0] ?? "").byteLength;
  const push = (value: string, encodedLength = utf8Length(value)): void => {
    bytes += encodedLength;
    if (bytes > MAX_CANONICAL_DATA_BYTES) throw new Error("PrintPacket canonical data exceeds 4 MiB");
    chunks.push(value);
  };
  const string = (value: string): void => {
    const length = utf8Length(value);
    push(`s${length}:${value}`, 2 + String(length).length + length);
  };
  const visit = (value: JsonValue, depth: number): void => {
    if (depth > MAX_CANONICAL_DATA_DEPTH) throw new Error("PrintPacket data exceeds 128 levels");
    if (value === null) { push("n"); return; }
    if (value === false) { push("f"); return; }
    if (value === true) { push("t"); return; }
    if (typeof value === "string") { string(value); return; }
    if (typeof value === "number") { push(`d${canonicalNumber(value)}`); return; }
    if (Array.isArray(value)) {
      push(`a${value.length}:`);
      for (const item of value) visit(item, depth + 1);
      return;
    }
    const keys = Object.keys(value).sort(compareUtf8);
    push(`o${keys.length}:`);
    for (const key of keys) {
      string(key);
      visit(value[key] as JsonValue, depth + 1);
    }
  };
  visit(data, 0);
  return encoder.encode(chunks.join(""));
}

export function canonicalData(data: JsonObject): string {
  return decoder.decode(canonicalDataBytes(data));
}
function hex(bytes: ArrayBuffer): string { return [...new Uint8Array(bytes)].map((byte) => byte.toString(16).padStart(2, "0")).join(""); }
function canonicalTarget(target: OutputTarget): string {
  return target.kind === "pdf"
    ? JSON.stringify({ kind: target.kind, profile: target.profile })
    : JSON.stringify({ kind: target.kind, language: target.language, profile: target.profile, dpi: target.dpi, printable_width_dots: target.printable_width_dots });
}
export async function renderCacheKey(manifest: TemplateManifest, data: JsonObject, target: OutputTarget = { kind: "pdf", profile: PDF_BASE14_V1 }): Promise<string> {
  const prefix = encoder.encode(`printpacket.render-cache/v1\0${manifest.canonical_sha256}\0${CONFORMANCE_CORE_V1}\0${canonicalTarget(target)}\0`);
  const dataBytes = canonicalDataBytes(data);
  const input = new Uint8Array(prefix.byteLength + dataBytes.byteLength);
  input.set(prefix);
  input.set(dataBytes, prefix.byteLength);
  return hex(await crypto.subtle.digest("SHA-256", input));
}
