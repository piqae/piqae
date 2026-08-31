import { canonicalToLiquid } from "./liquid-document-adapter";
import {
  serializeTemplateEnvelope,
  type Block,
  type PrintPacket,
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
const currentValue = (parts: string[], bold = false): Block => ({
  type: "paragraph",
  content: [
    {
      type: "value",
      value: current(...parts),
      style: bold ? { bold: true } : {},
    },
  ],
});
const optionalCurrentValue = (parts: string[], bold = false): Block => ({
  type: "conditional",
  condition: { type: "exists", value: current(...parts) },
  then: [currentValue(parts, bold)],
  else: [],
});
const nonEmptyCurrent = (...parts: string[]) => ({
  type: "compare" as const,
  operator: "not_equal" as const,
  left: {
    type: "format_string" as const,
    operation: "trim" as const,
    value: {
      type: "coalesce" as const,
      values: [current(...parts), literal("")],
    },
  },
  right: literal(""),
});
const currentMoney = (...parts: string[]) => ({
  type: "format_money" as const,
  amount: current(...parts),
  currency: current("currency"),
});
const column = (
  label: string,
  expression:
    | ReturnType<typeof path>
    | ReturnType<typeof current>
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
  column("SKU", current("sku"), 2),
  column("Qty", current("quantity"), 1, "right"),
  column("Price", currentMoney("unitPrice"), 2, "right"),
  column("Total", currentMoney("total"), 2, "right"),
];
const items = (source = "lineItems", columns = itemColumns()): Block => ({
  type: "table",
  items: current(source),
  repeat_header: true,
  columns,
  empty: [text("No items")],
});
const region = () => ({
  first: [] as Block[],
  default: [] as Block[],
  last: [] as Block[],
});
const base = (body: Block[], continuous = false): PrintPacket => ({
  format: "printpacket/v1",
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
  footer: region(),
});
const label = (body: Block[]): PrintPacket => ({
  ...base(body),
  media: {
    kind: "label",
    width_mm: 100,
    height_mm: 50,
    margins: { top_mm: 2, right_mm: 2, bottom_mm: 2, left_mm: 2 },
  },
  theme: {
    font_size_pt: 9,
    line_height: 1.15,
    text_color: { red: 32, green: 34, blue: 35 },
  },
});
const documents = {
  invoice: base([
    {
      type: "repeat",
      items: path("orders"),
      gap_mm: 10,
      children: [
        {
          type: "grid",
          columns: [2, 1],
          gap_mm: 8,
          children: [value(["shop", "name"], true), text("INVOICE", 1)],
        },
        currentValue(["name"], true),
        optionalCurrentValue(["shippingAddress", "formatted"]),
        { type: "divider" },
        items(),
        { type: "divider" },
        {
          type: "paragraph",
          content: [
            { type: "text", value: "Subtotal " },
            { type: "value", value: currentMoney("subtotal") },
          ],
        },
        {
          type: "paragraph",
          content: [
            { type: "text", value: "Tax " },
            { type: "value", value: currentMoney("tax") },
          ],
        },
        {
          type: "paragraph",
          content: [
            { type: "text", value: "Total ", style: { bold: true } },
            {
              type: "value",
              value: currentMoney("total"),
              style: { bold: true },
            },
          ],
        },
        { type: "page_break" },
      ],
    },
  ]),
  "packing-slip": base([
    {
      type: "repeat",
      items: path("orders"),
      gap_mm: 10,
      children: [
        {
          type: "grid",
          columns: [2, 1],
          gap_mm: 10,
          children: [
            {
              type: "stack",
              gap_mm: 3,
              children: [
                value(["shop", "name"], true),
                {
                  type: "conditional",
                  condition: nonEmptyCurrent("statusUrl"),
                  then: [
                    {
                      type: "qr",
                      value: current("statusUrl"),
                      size_mm: 24,
                    },
                  ],
                  else: [],
                },
              ],
            },
            {
              type: "stack",
              gap_mm: 1,
              children: [
                text("PACKING SLIP", 1),
                currentValue(["name"], true),
                currentValue(["createdAt"]),
              ],
            },
          ],
        },
        { type: "spacer", height_mm: 7 },
        {
          type: "grid",
          columns: [1, 1],
          gap_mm: 12,
          children: [
            {
              type: "stack",
              gap_mm: 1,
              children: [
                text("SHIP TO", 2),
                optionalCurrentValue(["shippingAddress", "formatted"]),
              ],
            },
            {
              type: "stack",
              gap_mm: 1,
              children: [
                text("CUSTOMER", 2),
                optionalCurrentValue(["customer", "displayName"]),
                optionalCurrentValue(["customer", "email"]),
              ],
            },
          ],
        },
        { type: "spacer", height_mm: 6 },
        { type: "divider", width_pt: 1 },
        items("lineItems", itemColumns().slice(0, 3)),
        { type: "divider" },
        text("Thank you for your order."),
        currentValue(["name"]),
        { type: "page_break" },
      ],
    },
  ]),
  receipt: base(
    [
      value(["shop", "name"], true),
      {
        type: "repeat",
        items: path("orders"),
        gap_mm: 6,
        children: [
          currentValue(["name"], true),
          {
            type: "paragraph",
            content: [
              {
                type: "value",
                value: {
                  type: "format_date",
                  value: current("createdAt"),
                  format: "day_month_year",
                },
              },
            ],
          },
          { type: "divider" },
          items("lineItems", [
            column("Item", current("title"), 5),
            column("Qty", current("quantity"), 1, "right"),
            column("Total", currentMoney("total"), 2, "right"),
          ]),
          { type: "divider" },
          {
            type: "paragraph",
            content: [
              { type: "text", value: "Tax " },
              { type: "value", value: currentMoney("tax") },
            ],
          },
          {
            type: "paragraph",
            content: [
              { type: "text", value: "Total ", style: { bold: true } },
              {
                type: "value",
                value: currentMoney("total"),
                style: { bold: true },
              },
            ],
          },
          text("Thank you."),
        ],
      },
    ],
    true,
  ),
  "product-label": label([
    {
      type: "repeat",
      items: path("orders"),
      children: [
        {
          type: "repeat",
          items: current("lineItems"),
          children: [
            {
              type: "keep_together",
              children: [
                {
                  type: "stack",
                  gap_mm: 0.5,
                  children: [
                    currentValue(["title"], true),
                    optionalCurrentValue(["variant", "title"]),
                    {
                      type: "paragraph",
                      content: [
                        {
                          type: "value",
                          value: currentMoney("unitPrice"),
                          style: { bold: true, font_size_pt: 13 },
                        },
                      ],
                    },
                  ],
                },
                { type: "spacer", height_mm: 1 },
                {
                  type: "conditional",
                  condition: {
                    type: "exists",
                    value: current("labelCode128"),
                  },
                  then: [
                    {
                      type: "barcode",
                      value: current("labelCode128"),
                      symbology: "code128",
                      width_mm: 88,
                      height_mm: 16,
                      human_readable: true,
                    },
                  ],
                  else: [],
                },
              ],
            },
            { type: "page_break" },
          ],
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
  specification: PrintPacket;
  kind: "invoice" | "packing_slip" | "receipt" | "credit_note" | "label";
  pageSize: "A4" | "80mm" | "100x50mm";
  source: string;
};
export const starterTemplates: readonly StarterTemplate[] = Object.entries(
  documents,
).map(([id, specification]) => {
  const kind =
    id === "packing-slip"
      ? "packing_slip"
      : id === "product-label"
        ? "label"
        : id === "receipt"
          ? "receipt"
          : "invoice";
  const liquid = canonicalToLiquid(specification);
  return {
    id,
    name: id
      .split("-")
      .map((x) => x[0]!.toUpperCase() + x.slice(1))
      .join(" "),
    details: `${kind.replaceAll("_", " ")} · ${
      specification.media.kind === "continuous"
        ? "80 mm"
        : specification.media.kind === "label"
          ? "100 × 50 mm"
          : "A4"
    }`,
    status: "Published",
    specification,
    kind,
    pageSize:
      specification.media.kind === "continuous"
        ? "80mm"
        : specification.media.kind === "label"
          ? "100x50mm"
          : "A4",
    source: serializeTemplateEnvelope({
      schema: "piqae.shopify-printpacket-template/v1",
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
