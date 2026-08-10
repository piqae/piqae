import type { DocumentSpec } from "@piqae/sdk";
import { canonicalToLiquid } from "./liquid-document-adapter";
import {
  serializeTemplateEnvelope,
  visualToCanonical,
  type PdfmeVisualField,
  type PdfmeVisualModel,
} from "./template-model";

export type StarterTemplate = {
  id: string;
  name: string;
  details: string;
  status: "Published" | "Draft";
  specification: DocumentSpec;
  kind: "invoice" | "packing_slip" | "receipt";
  pageSize: "A4" | "80mm";
  source: string;
};

const text = (
  id: string,
  x: number,
  y: number,
  width: number,
  height: number,
  value: { binding: string } | { text: string },
  fontSize = 10,
): PdfmeVisualField => ({
  id,
  type: "text",
  x,
  y,
  width,
  height,
  fontSize,
  ...value,
});
const line = (
  id: string,
  x: number,
  y: number,
  width: number,
): PdfmeVisualField => ({
  id,
  type: "line",
  x,
  y,
  width,
  height: 0.3,
});

/**
 * A supported, asset-free adaptation of pdfme's MIT-licensed official invoice
 * example. Unsupported SVG, expression and table plugins are intentionally not
 * copied; Piqae bindings remain deterministic and editable in both views.
 * https://github.com/pdfme/pdfme/tree/main/playground/public/template-assets/invoice
 */
const invoiceFields: PdfmeVisualField[] = [
  text("brand", 20, 20, 85, 12, { binding: "/shop" }, 15),
  text("heading", 120, 20, 70, 23, { text: "INVOICE" }, 36),
  text("billed-label", 20, 58, 84, 8, { text: "Billed to" }, 12),
  text(
    "customer",
    20,
    68,
    85,
    10,
    { binding: "/orders/0/customer/displayName" },
    11,
  ),
  text("email", 20, 80, 85, 10, { binding: "/orders/0/customer/email" }, 10),
  text("invoice-label", 120, 58, 30, 8, { text: "Order" }, 11),
  text("order", 155, 58, 35, 8, { binding: "/orders/0/name" }, 11),
  text("date-label", 120, 70, 30, 8, { text: "Date" }, 11),
  text("date", 155, 70, 35, 8, { binding: "/orders/0/createdAt" }, 10),
  line("rule", 20, 105, 170),
  text("subtotal-label", 130, 154, 28, 8, { text: "Subtotal" }, 11),
  text("subtotal", 160, 154, 30, 8, { binding: "/orders/0/subtotal" }, 11),
  text("tax-label", 130, 165, 28, 8, { text: "Tax" }, 11),
  text("tax", 160, 165, 30, 8, { binding: "/orders/0/tax" }, 11),
  line("total-rule", 130, 176, 60),
  text("total-label", 130, 181, 28, 12, { text: "Total" }, 18),
  text("total", 160, 181, 30, 12, { binding: "/orders/0/total" }, 18),
  text("thanks", 20, 260, 170, 10, { text: "Thank you for your order." }, 10),
];

const packingFields: PdfmeVisualField[] = [
  text("heading", 15, 15, 120, 16, { text: "PACKING SLIP" }, 24),
  text("order", 150, 18, 45, 10, { binding: "/orders/0/name" }, 12),
  line("rule", 15, 38, 180),
  text("ship-label", 15, 48, 70, 8, { text: "Ship to" }, 12),
  text(
    "ship-name",
    15,
    60,
    85,
    10,
    { binding: "/orders/0/shippingAddress/name" },
    11,
  ),
  text(
    "ship-address",
    15,
    73,
    120,
    10,
    { binding: "/orders/0/shippingAddress/address1" },
    10,
  ),
  text(
    "ship-city",
    15,
    86,
    120,
    10,
    { binding: "/orders/0/shippingAddress/city" },
    10,
  ),
  text("created-label", 145, 48, 50, 8, { text: "Created" }, 11),
  text("created", 145, 60, 50, 10, { binding: "/orders/0/createdAt" }, 10),
  line("items-rule", 15, 110, 180),
  text("items-note", 15, 118, 180, 10, { text: "Items for this order" }, 12),
  text("footer", 15, 270, 180, 10, { text: "Packed with care." }, 10),
];

const receiptFields: PdfmeVisualField[] = [
  text("brand", 4, 5, 72, 9, { binding: "/shop" }, 14),
  text("receipt", 4, 17, 72, 8, { text: "RECEIPT" }, 12),
  text("order", 4, 29, 72, 8, { binding: "/orders/0/name" }, 10),
  text("date", 4, 39, 72, 8, { binding: "/orders/0/createdAt" }, 9),
  line("rule", 4, 50, 72),
  text("subtotal-label", 4, 58, 35, 8, { text: "Subtotal" }, 10),
  text("subtotal", 45, 58, 31, 8, { binding: "/orders/0/subtotal" }, 10),
  text("tax-label", 4, 69, 35, 8, { text: "Tax" }, 10),
  text("tax", 45, 69, 31, 8, { binding: "/orders/0/tax" }, 10),
  line("total-rule", 4, 80, 72),
  text("total-label", 4, 87, 35, 10, { text: "TOTAL" }, 13),
  text("total", 45, 87, 31, 10, { binding: "/orders/0/total" }, 13),
  text("thanks", 4, 110, 72, 8, { text: "Thank you." }, 10),
];

function starter(
  key: StarterTemplate["id"],
  name: string,
  details: string,
  kind: StarterTemplate["kind"],
  page: PdfmeVisualModel["page"],
  fields: PdfmeVisualField[],
): StarterTemplate {
  const visual: PdfmeVisualModel = {
    schema: "pdfme-compatible/v1",
    page,
    fields,
  };
  const specification = visualToCanonical(visual);
  const liquid = canonicalToLiquid(specification);
  if (!liquid.source)
    throw new Error("Starter template is not Liquid compatible");
  return {
    id: key,
    name,
    details,
    status: "Published",
    kind,
    pageSize: page as StarterTemplate["pageSize"],
    specification,
    source: serializeTemplateEnvelope({
      schema: "piqae.shopify-template/v1",
      canonical: specification,
      editor: {
        mode: "visual",
        pdfme: visual,
        liquid: liquid.source,
        roundTrip: "lossless",
        warnings: [],
      },
      assets: [],
      system: { key, immutable: true },
    }),
  };
}

export const starterTemplates: readonly StarterTemplate[] = [
  starter("invoice", "Invoice", "Orders · A4", "invoice", "A4", invoiceFields),
  starter(
    "packing-slip",
    "Packing slip",
    "Fulfillment · A4",
    "packing_slip",
    "A4",
    packingFields,
  ),
  starter(
    "receipt",
    "Receipt",
    "Orders · 80 mm",
    "receipt",
    "80mm",
    receiptFields,
  ),
] as const;
