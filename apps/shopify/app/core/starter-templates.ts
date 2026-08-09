import type { DocumentSpec } from "@piqae/sdk";
import {
  serializeTemplateEnvelope,
  type PdfmeVisualModel,
} from "./template-model";

export type StarterTemplate = {
  id: string;
  name: string;
  details: string;
  status: "Published" | "Draft";
  specification: DocumentSpec;
  kind:
    | "invoice"
    | "packing_slip"
    | "receipt"
    | "returns"
    | "credit_note"
    | "custom";
  pageSize: "A4" | "A5" | "80mm";
  source: string;
};

const orderRows: DocumentSpec["body"] = [
  { type: "text", value: { pointer: "/shop/name" }, font_size: 10 },
  { type: "text", value: { pointer: "/document/title" }, font_size: 22 },
  { type: "text", value: { pointer: "/order/name" }, font_size: 12 },
  { type: "line" },
  {
    type: "repeat",
    pointer: "/order/line_items",
    children: [
      {
        type: "row",
        gap_mm: 4,
        children: [
          { type: "text", value: { pointer: "./title" } },
          { type: "text", value: { pointer: "./quantity" } },
          { type: "text", value: { pointer: "./total" } },
        ],
      },
    ],
  },
  { type: "line" },
  { type: "text", value: { pointer: "/order/total" }, font_size: 13 },
];

function document(size: DocumentSpec["page"]["size"] = "a4"): DocumentSpec {
  return {
    spec_version: "piqae.document/v1",
    page: { size, margin_mm: size === "roll80mm" ? 4 : 10 },
    body: structuredClone(orderRows),
  };
}

function visual(page: PdfmeVisualModel["page"]): PdfmeVisualModel {
  return {
    schema: "pdfme-compatible/v1",
    page,
    fields: [
      {
        id: "title",
        type: "text",
        x: 10,
        y: 10,
        width: 100,
        height: 12,
        binding: "/document/title",
      },
      {
        id: "order",
        type: "text",
        x: 10,
        y: 28,
        width: 80,
        height: 8,
        binding: "/order/name",
      },
    ],
  };
}

function starter(
  key: string,
  name: string,
  details: string,
  kind: StarterTemplate["kind"],
  pageSize: StarterTemplate["pageSize"],
  specification: DocumentSpec,
): StarterTemplate {
  return {
    id: key,
    name,
    details,
    status: "Published",
    kind,
    pageSize,
    specification,
    source: serializeTemplateEnvelope({
      schema: "piqae.shopify-template/v1",
      canonical: specification,
      editor: {
        mode: "visual",
        pdfme: visual(pageSize),
        roundTrip: "lossless",
        warnings: [],
      },
      assets: [],
      system: { key, immutable: true },
    }),
  };
}

export const starterTemplates: readonly StarterTemplate[] = [
  starter("invoice", "Invoice", "Orders · A4", "invoice", "A4", document()),
  starter(
    "packing-slip",
    "Packing slip",
    "Fulfillment · A4",
    "packing_slip",
    "A4",
    document(),
  ),
  starter(
    "receipt",
    "Receipt",
    "Orders · 80 mm",
    "receipt",
    "80mm",
    document("roll80mm"),
  ),
  starter(
    "returns-form",
    "Returns form",
    "Returns · A4",
    "returns",
    "A4",
    document(),
  ),
  starter(
    "quote-pro-forma",
    "Quote / pro forma",
    "Draft orders · A4",
    "custom",
    "A4",
    document(),
  ),
  starter(
    "credit-note",
    "Refund / credit note",
    "Refunds · A4",
    "credit_note",
    "A4",
    document(),
  ),
  starter(
    "gift-receipt",
    "Gift receipt",
    "Orders · A5",
    "receipt",
    "A5",
    document("a5"),
  ),
  starter(
    "delivery-note",
    "Delivery note",
    "Fulfillment · A4",
    "packing_slip",
    "A4",
    document(),
  ),
] as const;
