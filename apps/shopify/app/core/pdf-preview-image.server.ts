import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { getDocument } from "pdfjs-dist/legacy/build/pdf.mjs";

export const MAX_PREVIEW_PDF_BYTES = 32 * 1024 * 1024;
const MAX_PREVIEW_PIXELS = 2_000_000;
const TARGET_PREVIEW_WIDTH = 900;
const MAX_PREVIEW_SCALE = 2;
const require = createRequire(import.meta.url);
const STANDARD_FONT_DATA_URL = resolve(
  dirname(require.resolve("pdfjs-dist/legacy/build/pdf.mjs")),
  "../../standard_fonts/",
).concat("/");

type ServerCanvas = {
  toBuffer(type: "image/png"): Uint8Array;
};
type ServerCanvasContext = {
  canvas: ServerCanvas;
};
type ServerCanvasFactory = {
  create(
    width: number,
    height: number,
  ): {
    canvas: ServerCanvas;
    context: ServerCanvasContext;
  };
  destroy(value: { canvas: ServerCanvas; context: ServerCanvasContext }): void;
};

export async function readBoundedPdf(
  response: Response,
  limit = MAX_PREVIEW_PDF_BYTES,
): Promise<Uint8Array> {
  const length = Number(response.headers.get("content-length"));
  if (Number.isFinite(length) && length > limit)
    throw new Error("preview PDF exceeds the image preview limit");
  if (!response.body) throw new Error("preview PDF has no response body");

  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > limit) {
        await reader.cancel();
        throw new Error("preview PDF exceeds the image preview limit");
      }
      chunks.push(value);
    }
  } finally {
    reader.releaseLock();
  }

  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return bytes;
}

export async function renderFirstPdfPagePng(
  bytes: Uint8Array,
): Promise<Uint8Array> {
  if (
    bytes.byteLength < 5 ||
    new TextDecoder("ascii").decode(bytes.subarray(0, 5)) !== "%PDF-"
  )
    throw new Error("preview artifact is not a PDF");

  const task = getDocument({
    data: bytes,
    standardFontDataUrl: STANDARD_FONT_DATA_URL,
    useSystemFonts: false,
    useWasm: false,
  });
  try {
    const pdf = await task.promise;
    if (pdf.numPages < 1) throw new Error("preview PDF has no pages");
    const page = await pdf.getPage(1);
    try {
      const natural = page.getViewport({ scale: 1 });
      if (
        !Number.isFinite(natural.width) ||
        !Number.isFinite(natural.height) ||
        natural.width <= 0 ||
        natural.height <= 0
      )
        throw new Error("preview PDF page has invalid dimensions");
      let scale = Math.min(
        MAX_PREVIEW_SCALE,
        TARGET_PREVIEW_WIDTH / natural.width,
      );
      const pixels = natural.width * natural.height * scale * scale;
      if (pixels > MAX_PREVIEW_PIXELS)
        scale *= Math.sqrt(MAX_PREVIEW_PIXELS / pixels);
      const viewport = page.getViewport({ scale });
      const canvasFactory = pdf.canvasFactory as ServerCanvasFactory;
      const rendered = canvasFactory.create(
        Math.max(1, Math.ceil(viewport.width)),
        Math.max(1, Math.ceil(viewport.height)),
      );
      try {
        await page.render({
          canvas: null,
          canvasContext:
            rendered.context as unknown as CanvasRenderingContext2D,
          viewport,
        }).promise;
        return rendered.canvas.toBuffer("image/png");
      } finally {
        canvasFactory.destroy(rendered);
      }
    } finally {
      page.cleanup();
    }
  } finally {
    await task.destroy();
  }
}
