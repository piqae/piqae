import { describe, expect, it } from "vitest";
import type { DocumentSpec } from "@piqae/sdk";
import {
  canonicalToLiquid,
  liquidToCanonical,
} from "../app/core/liquid-document-adapter";

const page: DocumentSpec["page"] = { size: "a4", margin_mm: 10 };

describe("bounded Liquid document adapter", () => {
  it("converts supported variables, loops, conditions and document tags", () => {
    const result = liquidToCanonical(
      [
        "Invoice",
        "{{ order.name }}",
        "{% if order.note %}",
        "{{ order.note }}",
        "{% endif %}",
        "{% for item in order.line_items %}",
        "{{ item.title }}",
        "{{ item.quantity }}",
        "{% endfor %}",
        "{% piqae_line %}",
        "{% piqae_qr order.status_url size_mm: 24 %}",
        "{% piqae_spacer 4 %}",
        "{% piqae_page_break %}",
      ].join("\n"),
      page,
    );
    expect(result.ok).toBe(true);
    if (!result.ok) return;
    expect(result.document.body).toMatchObject([
      { type: "text", value: "Invoice" },
      { type: "text", value: { pointer: "/order/name" } },
      { type: "when", pointer: "/order/note" },
      {
        type: "repeat",
        pointer: "/order/line_items",
        children: [
          { type: "text", value: { pointer: "./title" } },
          { type: "text", value: { pointer: "./quantity" } },
        ],
      },
      { type: "line" },
      { type: "qr", value: { pointer: "/order/status_url" }, size_mm: 24 },
      { type: "spacer", height_mm: 4 },
      { type: "page_break" },
    ]);
  });

  it("round-trips every representable canonical node deterministically", () => {
    const document: DocumentSpec = {
      spec_version: "piqae.document/v1",
      page,
      body: [
        { type: "text", value: "Packing slip" },
        {
          type: "repeat",
          pointer: "/order/line_items",
          children: [{ type: "text", value: { pointer: "./title" } }],
        },
        {
          type: "when",
          pointer: "/order/note",
          children: [{ type: "text", value: { pointer: "/order/note" } }],
        },
        { type: "qr", value: { pointer: "/order/status_url" }, size_mm: 20 },
      ],
    };
    const encoded = canonicalToLiquid(document);
    expect(encoded.diagnostics).toEqual([]);
    const decoded = liquidToCanonical(encoded.source!, page);
    expect(decoded).toMatchObject({ ok: true, document });
  });

  it("fails closed with stable line diagnostics for executable Liquid and HTML", () => {
    for (const [source, code] of [
      ["{{ order.name | escape }}", "unsupported_construct"],
      ["{% include 'remote' %}", "unsupported_construct"],
      ["<img src=https://example.test/a.png>", "unsupported_construct"],
      ["{% assign x = 1 %}", "unsupported_construct"],
    ]) {
      expect(liquidToCanonical(source, page)).toEqual({
        ok: false,
        diagnostics: [expect.objectContaining({ code, line: 1 })],
      });
    }
  });

  it("reports canonical structures that cannot be represented", () => {
    const document: DocumentSpec = {
      spec_version: "piqae.document/v1",
      page,
      body: [
        {
          type: "row",
          children: [{ type: "text", value: "not silently flattened" }],
        },
      ],
    };
    expect(canonicalToLiquid(document)).toEqual({
      diagnostics: [
        expect.objectContaining({
          code: "unsupported_node",
          message: expect.stringContaining("row"),
        }),
      ],
    });
  });
});
