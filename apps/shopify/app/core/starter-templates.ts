import type { DocumentSpec } from "@piqae/sdk";

export type StarterTemplate = {
  id: string;
  name: string;
  details: string;
  status: "Published" | "Draft";
  specification: DocumentSpec;
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

export const starterTemplates: readonly StarterTemplate[] = [
  {
    id: "invoice",
    name: "Invoice",
    details: "Orders · A4",
    status: "Published",
    specification: document(),
  },
  {
    id: "packing-slip",
    name: "Packing slip",
    details: "Fulfillment · A4",
    status: "Published",
    specification: document(),
  },
  {
    id: "receipt",
    name: "Receipt",
    details: "Orders · 80 mm",
    status: "Published",
    specification: document("roll80mm"),
  },
  {
    id: "returns-form",
    name: "Returns form",
    details: "Returns · A4",
    status: "Published",
    specification: document(),
  },
  {
    id: "quote-pro-forma",
    name: "Quote / pro forma",
    details: "Draft orders · A4",
    status: "Published",
    specification: document(),
  },
  {
    id: "credit-note",
    name: "Refund / credit note",
    details: "Refunds · A4",
    status: "Published",
    specification: document(),
  },
  {
    id: "gift-receipt",
    name: "Gift receipt",
    details: "Orders · A5",
    status: "Published",
    specification: document("a5"),
  },
  {
    id: "delivery-note",
    name: "Delivery note",
    details: "Fulfillment · A4",
    status: "Published",
    specification: document(),
  },
] as const;
