import { describe, expect, it } from "vitest";
import {
  canonicalToLiquid,
  liquidToCanonical,
} from "../app/core/liquid-document-adapter";
import { starterTemplates } from "../app/core/starter-templates";
describe("bounded Shopify Liquid profile", () => {
  it("compiles mixed text, filters, loops, conditions and tables", () => {
    const result = liquidToCanonical(
      "Invoice {{ order.name }}\n{% if order.taxTotal > 0 %}Tax {{ order.taxTotal | money }}{% endif %}\n{% piqae_table order.lineItems as: line %}",
    );
    expect(result.ok).toBe(true);
    if (result.ok)
      expect(result.document.body.map((x) => x.type)).toContain("table");
  });
  it("round trips all dynamic starters", () => {
    for (const starter of starterTemplates) {
      const liquid = canonicalToLiquid(starter.specification);
      const parsed = liquidToCanonical(
        liquid.source,
        starter.specification.media,
      );
      expect(parsed.ok).toBe(true);
    }
  });
  it("round trips unless without reversing its meaning", () => {
    const compiled = liquidToCanonical(
      "{% unless order.taxTotal %}No tax{% endunless %}",
    );
    expect(compiled.ok).toBe(true);
    if (!compiled.ok) return;
    expect(compiled.normalizedSource).toContain("{% unless order.taxTotal %}");
    const again = liquidToCanonical(compiled.normalizedSource);
    expect(again.ok).toBe(true);
    if (again.ok) expect(again.document).toEqual(compiled.document);
  });
  it("rejects executable and presentation constructs precisely", () => {
    const html = liquidToCanonical("<style>*{}</style>");
    expect(html.ok).toBe(false);
    if (!html.ok)
      expect(html.diagnostics[0]).toMatchObject({
        code: "html_unsupported",
        line: 1,
      });
    const include = liquidToCanonical("{% render 'secret' %}");
    expect(include.ok).toBe(false);
    if (!include.ok)
      expect(include.diagnostics[0]?.code).toBe("unsupported_tag");
  });
});
