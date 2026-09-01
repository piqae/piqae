import { describe, expect, it } from "vitest";

import {
  readBoundedPdf,
  renderFirstPdfPagePng,
} from "../app/core/pdf-preview-image.server";

function onePagePdf(): Uint8Array {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
    "<< /Length 42 >>\nstream\nBT /F1 18 Tf 20 50 Td (Piqae) Tj ET\nendstream",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  let source = "%PDF-1.4\n";
  const offsets = [0];
  objects.forEach((object, index) => {
    offsets.push(new TextEncoder().encode(source).byteLength);
    source += `${index + 1} 0 obj\n${object}\nendobj\n`;
  });
  const xref = new TextEncoder().encode(source).byteLength;
  source += `xref\n0 ${objects.length + 1}\n`;
  source += "0000000000 65535 f \n";
  for (const offset of offsets.slice(1))
    source += `${offset.toString().padStart(10, "0")} 00000 n \n`;
  source += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return new TextEncoder().encode(source);
}

describe("PDF preview images", () => {
  it("bounds streamed preview artifacts", async () => {
    await expect(
      readBoundedPdf(new Response(new Uint8Array([1, 2, 3, 4])), 3),
    ).rejects.toThrow("exceeds the image preview limit");
  });

  it("renders the exact first PDF page as a bounded PNG", async () => {
    const image = await renderFirstPdfPagePng(onePagePdf());
    expect([...image.subarray(0, 8)]).toEqual([
      137, 80, 78, 71, 13, 10, 26, 10,
    ]);
    expect(image.byteLength).toBeGreaterThan(100);
  });

  it("rejects non-PDF input", async () => {
    await expect(
      renderFirstPdfPagePng(new TextEncoder().encode("not a PDF")),
    ).rejects.toThrow("not a PDF");
  });
});
