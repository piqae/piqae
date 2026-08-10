import { describe, expect, it, vi } from "vitest";

import {
  authorizedJson,
  buildAdminPrintUrl,
  buildDraftPrintUrl,
  buildPosPrintUrl,
  printPosReceipt,
} from "./print-urls.js";

describe("print URL contracts", () => {
  it("preserves Headers instances while adding Shopify authorization", async () => {
    vi.stubGlobal("shopify", {
      auth: { idToken: vi.fn().mockResolvedValue("session-token") },
    });
    const fetcher = vi.fn().mockResolvedValue(Response.json({ ok: true }));
    vi.stubGlobal("fetch", fetcher);
    await authorizedJson("/api/example", {
      headers: new Headers([
        ["content-type", "application/json"],
        ["x-request-id", "request-1"],
      ]),
    });
    const headers = fetcher.mock.calls[0][1].headers;
    expect(headers).toBeInstanceOf(Headers);
    expect(headers.get("content-type")).toBe("application/json");
    expect(headers.get("x-request-id")).toBe("request-1");
    expect(headers.get("authorization")).toBe("Bearer session-token");
    expect(headers.get("accept")).toBe("application/json");
    vi.unstubAllGlobals();
  });
  it("builds an encoded bulk admin PDF URL and removes duplicate document types", () => {
    const url = buildAdminPrintUrl({
      orderIds: ["gid://shopify/Order/1", "gid://shopify/Order/2"],
      documents: ["invoice", "invoice", "packing_slip"],
      templateId: "published-template-4",
    });
    const parsed = new URL(url, "https://app.example");
    expect(parsed.pathname).toBe("/api/print/admin");
    expect(parsed.searchParams.get("orderIds")).toBe(
      "gid://shopify/Order/1,gid://shopify/Order/2",
    );
    expect(parsed.searchParams.get("documents")).toBe("invoice,packing_slip");
    expect(parsed.searchParams.get("format")).toBe("pdf");
    expect(parsed.searchParams.get("templateId")).toBe("published-template-4");
  });

  it("does not produce a printable URL without a selected resource and document", () => {
    expect(
      buildAdminPrintUrl({ orderIds: [], documents: ["invoice"] }),
    ).toBeNull();
    expect(
      buildAdminPrintUrl({
        orderIds: ["gid://shopify/Order/1"],
        documents: [],
      }),
    ).toBeNull();
    expect(buildPosPrintUrl({ orderId: -1, format: "html" })).toBeNull();
  });

  it("builds a deduplicated draft-order action URL", () => {
    const url = buildDraftPrintUrl({
      draftOrderIds: [
        "gid://shopify/DraftOrder/1",
        "gid://shopify/DraftOrder/1",
      ],
    });
    const parsed = new URL(url, "https://app.example");
    expect(parsed.pathname).toBe("/api/print/admin-drafts");
    expect(parsed.searchParams.get("draftOrderIds")).toBe(
      "gid://shopify/DraftOrder/1",
    );
    expect(buildDraftPrintUrl({ draftOrderIds: [] })).toBeNull();
  });
});

describe("POS printing", () => {
  it("sends HTML directly to a connected receipt printer", async () => {
    const printer = { id: "front", name: "Front counter", connected: true };
    const printing = {
      getPrinters: vi.fn().mockResolvedValue([printer]),
      print: vi.fn(),
    };
    await expect(
      printPosReceipt({ printing, orderId: 42, printer }),
    ).resolves.toEqual({
      mode: "receipt-printer",
      printer,
    });
    expect(printing.print).toHaveBeenCalledWith(
      expect.stringContaining("format=html"),
      { printer },
    );
  });

  it("requires an explicitly selected connected printer", async () => {
    const printing = {
      getPrinters: vi.fn().mockResolvedValue([]),
      print: vi.fn(),
    };
    await expect(printPosReceipt({ printing, orderId: 42 })).rejects.toThrow(
      "Select a connected receipt printer",
    );
    expect(printing.print).not.toHaveBeenCalled();
  });

  it("does not auto-fallback after an uncertain direct submission", async () => {
    const printer = { id: "front", name: "Front counter", connected: true };
    const printing = {
      getPrinters: vi.fn().mockResolvedValue([printer]),
      print: vi.fn().mockRejectedValueOnce(new Error("disconnected")),
    };
    await expect(
      printPosReceipt({ printing, orderId: 42, printer }),
    ).rejects.toThrow("disconnected");
    expect(printing.print).toHaveBeenCalledTimes(1);
    expect(printing.print).toHaveBeenCalledWith(
      expect.stringContaining("format=html"),
      { printer },
    );
  });

  it("rejects a disconnected selected printer without submitting", async () => {
    const printing = {
      print: vi.fn(),
    };
    await expect(
      printPosReceipt({
        printing,
        orderId: 42,
        printer: { id: "front", name: "Front", connected: false },
      }),
    ).rejects.toThrow("Select a connected receipt printer");
    expect(printing.print).not.toHaveBeenCalled();
  });

  it("rejects malformed order identifiers before submitting", async () => {
    const printing = { print: vi.fn() };
    await expect(
      printPosReceipt({
        printing,
        orderId: Number.MAX_SAFE_INTEGER + 1,
        printer: { id: "front", name: "Front", connected: true },
      }),
    ).rejects.toThrow("valid POS order");
    expect(printing.print).not.toHaveBeenCalled();
  });

  it("does not permit an attacker-controlled printer shape without an id", async () => {
    const printing = { print: vi.fn() };
    await expect(
      printPosReceipt({
        printing,
        orderId: 42,
        printer: { name: "Missing id", connected: true },
      }),
    ).rejects.toThrow("Select a connected receipt printer");
    expect(printing.print).not.toHaveBeenCalled();
  });
});
