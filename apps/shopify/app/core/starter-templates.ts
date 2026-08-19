import { canonicalToLiquid } from "./liquid-document-adapter";
import {
  serializeTemplateEnvelope,
  type Block,
  type BusinessDocument,
  type Inline,
} from "./template-model";
const path = (...parts: string[]) => ({ type: "path" as const, path: parts });
const current = (...parts: string[]) => ({
  type: "current_path" as const,
  path: parts,
});
const literal = (value: string) => ({ type: "literal" as const, value });
const text = (value: string, level?: 1 | 2 | 3): Block =>
  level
    ? { type: "heading", level, content: [{ type: "text", value }] }
    : { type: "paragraph", content: [{ type: "text", value }] };
const value = (parts: string[], bold = false): Block => ({
  type: "paragraph",
  content: [
    { type: "value", value: path(...parts), style: bold ? { bold: true } : {} },
  ],
});
const money = (...parts: string[]) => ({
  type: "format_money" as const,
  amount: path(...parts),
  currency: path("order", "currencyCode"),
});
const currentMoney = (...parts: string[]) => ({
  type: "format_money" as const,
  amount: current(...parts),
  currency: path("order", "currencyCode"),
});
const column = (
  label: string,
  expression:
    | ReturnType<typeof path>
    | ReturnType<typeof current>
    | ReturnType<typeof money>
    | ReturnType<typeof currentMoney>,
  width: number,
  align: "left" | "right" = "left",
) => ({
  header: [{ type: "text" as const, value: label, style: { bold: true } }],
  cell: [{ type: "value" as const, value: expression }],
  width,
  align,
});
const itemColumns = (): Extract<Block, { type: "table" }>["columns"] => [
  column("Item", current("title"), 5),
  column("Qty", current("quantity"), 1, "right"),
  column("Price", currentMoney("price"), 2, "right"),
  column("Total", currentMoney("total"), 2, "right"),
];
const items = (source = "lineItems", columns = itemColumns()): Block => ({
  type: "table",
  items: path("order", source),
  repeat_header: true,
  columns,
  empty: [text("No items")],
});
const region = () => ({
  first: [] as Block[],
  default: [] as Block[],
  last: [] as Block[],
});
const base = (body: Block[], continuous = false): BusinessDocument => ({
  format: "piqae.business-document/v1",
  media: continuous
    ? {
        kind: "continuous",
        width_mm: 80,
        margins: { top_mm: 4, right_mm: 4, bottom_mm: 4, left_mm: 4 },
      }
    : {
        kind: "paged",
        size: "a4",
        orientation: "portrait",
        margins: { top_mm: 14, right_mm: 14, bottom_mm: 16, left_mm: 14 },
      },
  theme: {
    font_size_pt: 10,
    line_height: 1.35,
    text_color: { red: 32, green: 34, blue: 35 },
  },
  resources: {},
  header: region(),
  body,
  footer: {
    ...region(),
    default: [
      {
        type: "paragraph",
        content: [
          { type: "text", value: "Page " },
          { type: "value", value: path("page", "number") },
        ],
      },
    ],
  },
});
const documents = {
  invoice: base([
    text("INVOICE", 1),
    value(["shop", "name"], true),
    value(["order", "name"]),
    value(["order", "billingAddress", "formatted"]),
    { type: "divider" },
    items(),
    { type: "divider" },
    {
      type: "paragraph",
      content: [
        { type: "text", value: "Subtotal " },
        { type: "value", value: money("order", "subtotal") },
      ],
    },
    {
      type: "paragraph",
      content: [
        { type: "text", value: "Tax " },
        { type: "value", value: money("order", "taxTotal") },
      ],
    },
    {
      type: "paragraph",
      content: [
        { type: "text", value: "Total ", style: { bold: true } },
        {
          type: "value",
          value: money("order", "total"),
          style: { bold: true },
        },
      ],
    },
    { type: "qr", value: path("order", "statusUrl"), size_mm: 24 },
  ]),
  "packing-slip": base([
    text("PACKING SLIP", 1),
    value(["order", "name"]),
    text("Ship to", 2),
    value(["order", "shippingAddress", "formatted"]),
    items("lineItems", itemColumns().slice(0, 2)),
    text("Packed with care."),
  ]),
  receipt: base(
    [
      value(["shop", "name"], true),
      text("RECEIPT", 2),
      value(["order", "name"]),
      items(),
      { type: "divider" },
      {
        type: "paragraph",
        content: [
          { type: "text", value: "TOTAL ", style: { bold: true } },
          {
            type: "value",
            value: money("order", "total"),
            style: { bold: true },
          },
        ],
      },
      { type: "qr", value: path("order", "statusUrl"), size_mm: 22 },
      text("Thank you."),
    ],
    true,
  ),
  "credit-note": base([
    text("CREDIT NOTE", 1),
    value(["shop", "name"]),
    value(["order", "name"]),
    items("refundLineItems"),
    {
      type: "paragraph",
      content: [
        { type: "text", value: "Refund total ", style: { bold: true } },
        {
          type: "value",
          value: money("order", "refundTotal"),
          style: { bold: true },
        },
      ],
    },
  ]),
} as const;
void literal;
void (null as unknown as Inline);
export type StarterTemplate = {
  id: string;
  name: string;
  details: string;
  status: "Published";
  specification: BusinessDocument;
  kind: "invoice" | "packing_slip" | "receipt" | "credit_note";
  pageSize: "A4" | "80mm";
  source: string;
};
export const starterTemplates: readonly StarterTemplate[] = Object.entries(
  documents,
).map(([id, specification]) => {
  const kind =
    id === "packing-slip"
      ? "packing_slip"
      : id === "credit-note"
        ? "credit_note"
        : (id as "invoice" | "receipt");
  const liquid = canonicalToLiquid(specification);
  return {
    id,
    name: id
      .split("-")
      .map((x) => x[0]!.toUpperCase() + x.slice(1))
      .join(" "),
    details: `${kind.replaceAll("_", " ")} · ${specification.media.kind === "continuous" ? "80 mm" : "A4"}`,
    status: "Published",
    specification,
    kind,
    pageSize: specification.media.kind === "continuous" ? "80mm" : "A4",
    source: serializeTemplateEnvelope({
      schema: "piqae.shopify-business-template/v1",
      document: specification,
      editor: {
        mode: "visual",
        liquid: liquid.source,
        roundTrip: "lossless",
        warnings: [],
      },
      assets: [],
      system: { key: id, immutable: true },
    }),
  };
});
