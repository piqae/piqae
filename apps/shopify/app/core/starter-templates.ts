import { canonicalToLiquid } from "./liquid-document-adapter";
import {
  serializeTemplateEnvelope,
  type Block,
  type PrintPacket,
  type Inline,
  type TextStyle,
} from "./template-model";
const path = (...parts: string[]) => ({ type: "path" as const, path: parts });
const current = (...parts: string[]) => ({
  type: "current_path" as const,
  path: parts,
});
const literal = (value: string) => ({ type: "literal" as const, value });
const muted = { red: 102, green: 106, blue: 110 } as const;
const rule = { red: 210, green: 213, blue: 216 } as const;
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
const paragraph = (
  content: Inline[],
  style: Extract<Block, { type: "paragraph" }>["style"] = {},
): Block => ({ type: "paragraph", content, style });
const pathValue = (parts: string[], style: TextStyle = {}): Inline => ({
  type: "value",
  value: path(...parts),
  style,
});
const currentInline = (parts: string[], style: TextStyle = {}): Inline => ({
  type: "value",
  value: current(...parts),
  style,
});
const optionalCurrentInline = (
  parts: string[],
  style: TextStyle = {},
): Inline => ({
  type: "value",
  value: {
    type: "coalesce",
    values: [current(...parts), literal("")],
  },
  style,
});
const currentDate = (): Block =>
  paragraph(
    [
      {
        type: "value",
        value: {
          type: "format_date",
          value: current("createdAt"),
          format: "day_month_year",
        },
      },
    ],
    { align: "right", font_size_pt: 9 },
  );
const optionalCurrentValue = (parts: string[], bold = false): Block => ({
  type: "conditional",
  condition: { type: "exists", value: current(...parts) },
  then: [currentValue(parts, bold)],
  else: [],
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
const compactItemDescription = (): Inline[] => [
  currentInline(["title"], { bold: true }),
  { type: "line_break" },
  optionalCurrentInline(["variant", "title"], {
    font_size_pt: 8,
    color: muted,
  }),
  { type: "line_break" },
  { type: "text", value: "SKU: ", style: { font_size_pt: 8, color: muted } },
  optionalCurrentInline(["sku"], { font_size_pt: 8, color: muted }),
];
const documentTableStyle = {
  cell_padding_mm: 1.5,
  header_text_color: muted,
  border_color: rule,
  border_width_pt: 0.35,
} as const;
const packingItems = (): Block => ({
  type: "data_list",
  items: current("lineItems"),
  repeat_header: true,
  gap_mm: 0,
  header: [
    {
      type: "grid",
      columns: [9, 1],
      gap_mm: 1.5,
      children: [
        paragraph([{ type: "text", value: "ITEMS", style: { bold: true } }], {
          font_size_pt: 8,
          color: muted,
        }),
        paragraph([{ type: "text", value: "QTY", style: { bold: true } }], {
          align: "right",
          font_size_pt: 8,
          color: muted,
        }),
      ],
    },
    { type: "divider", width_pt: 0.35 },
  ],
  item: [
    {
      type: "grid",
      columns: [1.4, 7.6, 1],
      gap_mm: 1.5,
      children: [
        {
          type: "conditional",
          condition: { type: "exists", value: current("imageResource") },
          then: [
            {
              type: "image_value",
              resource: current("imageResource"),
              width_mm: 14,
              height_mm: 14,
              fit: "contain",
            },
          ],
          else: [],
        },
        paragraph(compactItemDescription()),
        paragraph([currentInline(["quantity"], { bold: true })], {
          align: "right",
        }),
      ],
    },
    { type: "divider", width_pt: 0.35 },
  ],
  empty: [text("No items")],
});
const invoiceColumns = (): Extract<Block, { type: "table" }>["columns"] => [
  {
    header: [
      { type: "text", value: "ITEMS", style: { bold: true, font_size_pt: 8 } },
    ],
    cell: compactItemDescription(),
    width: 6,
    align: "left",
  },
  {
    header: [
      { type: "text", value: "QTY", style: { bold: true, font_size_pt: 8 } },
    ],
    cell: [currentInline(["quantity"])],
    width: 1,
    align: "right",
  },
  {
    header: [
      { type: "text", value: "PRICE", style: { bold: true, font_size_pt: 8 } },
    ],
    cell: [{ type: "value", value: currentMoney("unitPrice") }],
    width: 2,
    align: "right",
  },
  {
    header: [
      { type: "text", value: "TOTAL", style: { bold: true, font_size_pt: 8 } },
    ],
    cell: [
      {
        type: "value",
        value: currentMoney("total"),
        style: { bold: true },
      },
    ],
    width: 2,
    align: "right",
  },
];
const items = (
  source = "lineItems",
  columns = itemColumns(),
  style?: Extract<Block, { type: "table" }>["style"],
): Block => ({
  type: "table",
  items: current(source),
  repeat_header: true,
  columns,
  empty: [text("No items")],
  ...(style ? { style } : {}),
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
const pagedDocument = (body: Block[]): PrintPacket => ({
  ...base(body),
  media: {
    kind: "paged",
    size: "a4",
    orientation: "portrait",
    margins: { top_mm: 13, right_mm: 13, bottom_mm: 14, left_mm: 13 },
  },
  theme: {
    font_size_pt: 9,
    line_height: 1.25,
    text_color: { red: 32, green: 34, blue: 35 },
  },
});
const brand = (): Block => ({
  type: "conditional",
  condition: { type: "exists", value: path("shop", "logo") },
  then: [
    {
      type: "image_value",
      resource: path("shop", "logo"),
      width_mm: 50,
      height_mm: 18,
      fit: "scale_down",
    },
  ],
  else: [
    paragraph([pathValue(["shop", "name"], { bold: true, font_size_pt: 18 })]),
  ],
});
const documentHeading = (
  title: string,
  barcode = false,
  referenceLabel = "Order ",
): Block => ({
  type: "grid",
  columns: [2, 1],
  gap_mm: 12,
  children: [
    brand(),
    {
      type: "stack",
      gap_mm: 0.8,
      children: [
        paragraph([{ type: "text", value: title, style: { bold: true } }], {
          align: "right",
          font_size_pt: 20,
        }),
        ...(barcode
          ? [
              {
                type: "conditional" as const,
                condition: {
                  type: "exists" as const,
                  value: current("referenceCode128"),
                },
                then: [
                  {
                    type: "barcode" as const,
                    value: current("referenceCode128"),
                    symbology: "code128" as const,
                    // Fill nearly the complete metadata column. Code128
                    // module widths remain uniform rather than being visually
                    // stretched, and unsuitable order references are omitted.
                    // Keep the complete footprint (bars + quiet padding)
                    // inside the narrow metadata column at every supported
                    // A4/Letter margin. The PDF renderer rejects overflow
                    // instead of silently scaling a barcode.
                    width_mm: 50,
                    height_mm: 10,
                    human_readable: false,
                    align: "right" as const,
                    padding_mm: 1.5,
                    gap_mm: 1.2,
                  },
                ],
                else: [],
              },
            ]
          : []),
        paragraph(
          [
            { type: "text", value: referenceLabel, style: { bold: true } },
            currentInline(["name"], { bold: true }),
          ],
          { align: "right", font_size_pt: 9 },
        ),
        currentDate(),
      ],
    },
  ],
});
const addressBlock = (label: string, parts: string[]): Block => ({
  type: "stack",
  gap_mm: 1,
  children: [
    paragraph([{ type: "text", value: label, style: { bold: true } }], {
      font_size_pt: 8,
      color: muted,
    }),
    optionalCurrentValue(parts),
  ],
});
const shopFooter = (message: string): Block => ({
  type: "stack",
  gap_mm: 0.6,
  children: [
    paragraph([{ type: "text", value: message }], {
      align: "center",
      font_size_pt: 8,
      color: muted,
    }),
    paragraph([pathValue(["shop", "name"], { bold: true })], {
      align: "center",
      font_size_pt: 8,
    }),
    {
      type: "conditional",
      condition: {
        type: "exists",
        value: path("shop", "address", "formatted"),
      },
      then: [
        paragraph([pathValue(["shop", "address", "formatted"])], {
          align: "center",
          font_size_pt: 8,
          color: muted,
        }),
      ],
      else: [],
    },
    {
      type: "conditional",
      condition: { type: "exists", value: path("shop", "email") },
      then: [
        paragraph([pathValue(["shop", "email"])], {
          align: "center",
          font_size_pt: 8,
          color: muted,
        }),
      ],
      else: [],
    },
    {
      type: "conditional",
      condition: { type: "exists", value: path("shop", "primaryDomain") },
      then: [
        paragraph([pathValue(["shop", "primaryDomain"])], {
          align: "center",
          font_size_pt: 8,
          color: muted,
        }),
      ],
      else: [],
    },
  ],
});
const totalRow = (label: string, parts: string[], bold = false): Block => ({
  type: "grid",
  columns: [1, 1],
  gap_mm: 4,
  children: [
    paragraph([{ type: "text", value: label, style: { bold } }], {
      font_size_pt: bold ? 10 : 9,
    }),
    paragraph(
      [
        {
          type: "value",
          value: currentMoney(...parts),
          style: { bold },
        },
      ],
      { align: "right", font_size_pt: bold ? 10 : 9 },
    ),
  ],
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
  invoice: pagedDocument([
    {
      type: "repeat",
      items: path("orders"),
      gap_mm: 10,
      children: [
        documentHeading("INVOICE", true, "Invoice "),
        { type: "spacer", height_mm: 10 },
        {
          type: "grid",
          columns: [1, 1],
          gap_mm: 14,
          children: [
            addressBlock("BILLING ADDRESS", ["billingAddress", "formatted"]),
            addressBlock("SHIPPING ADDRESS", ["shippingAddress", "formatted"]),
          ],
        },
        { type: "spacer", height_mm: 6 },
        items("lineItems", invoiceColumns(), documentTableStyle),
        { type: "spacer", height_mm: 4 },
        {
          type: "grid",
          columns: [3, 2],
          gap_mm: 10,
          children: [
            {
              type: "stack",
              gap_mm: 1,
              children: [
                paragraph(
                  [
                    {
                      type: "text",
                      value: "SHIPPING METHOD",
                      style: { bold: true },
                    },
                  ],
                  { font_size_pt: 8, color: muted },
                ),
                optionalCurrentValue(["shippingMethod"]),
                optionalCurrentValue(["note"]),
              ],
            },
            {
              type: "stack",
              gap_mm: 1,
              children: [
                totalRow("Subtotal", ["subtotal"]),
                totalRow("Tax", ["tax"]),
                { type: "divider", width_pt: 0.75 },
                totalRow("Total", ["total"], true),
              ],
            },
          ],
        },
        { type: "spacer", height_mm: 8 },
        shopFooter("Thank you for shopping with us!"),
        { type: "page_break" },
      ],
    },
  ]),
  "packing-slip": pagedDocument([
    {
      type: "repeat",
      items: path("orders"),
      gap_mm: 10,
      children: [
        documentHeading("PACKING SLIP", true),
        { type: "spacer", height_mm: 10 },
        {
          type: "grid",
          columns: [1, 1],
          gap_mm: 14,
          children: [
            addressBlock("SHIPPING ADDRESS", ["shippingAddress", "formatted"]),
            {
              type: "stack",
              gap_mm: 1,
              children: [
                paragraph(
                  [{ type: "text", value: "CUSTOMER", style: { bold: true } }],
                  { font_size_pt: 8, color: muted },
                ),
                optionalCurrentValue(["customer", "displayName"]),
                optionalCurrentValue(["customer", "email"]),
              ],
            },
          ],
        },
        { type: "spacer", height_mm: 6 },
        packingItems(),
        { type: "spacer", height_mm: 7 },
        shopFooter("Thank you for shopping with us!"),
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
                      align: "center",
                      padding_mm: 1.5,
                      gap_mm: 1.2,
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
