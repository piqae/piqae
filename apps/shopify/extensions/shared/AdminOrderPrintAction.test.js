import { describe, expect, it, vi } from "vitest";

import {
  chooseDefault,
  chooseDefaultDocument,
  loadWithTimeout,
  messageForLoadError,
  newInteractionId,
  stableOptionKey,
  PRINT_PLACEHOLDER_URL,
  canUseDestinationForPolicy,
  renderPolicySummary,
  targetForDocument,
  canUsePublishedBinding,
} from "./AdminOrderPrintAction.jsx";

describe("admin print action state", () => {
  it("creates bounded request IDs without Web Crypto", () => {
    const first = newInteractionId(["gid://shopify/Order/1004"]);
    const second = newInteractionId(["gid://shopify/Order/1004"]);
    expect(first).toMatch(/Order-1004-[a-z0-9]+-[a-z0-9]+$/);
    expect(second).not.toBe(first);
    expect(first.length).toBeLessThanOrEqual(128);
  });

  it("changes approval idempotency when a target specification changes", () => {
    expect(stableOptionKey("tgt_orders:spec_1")).toBe(
      stableOptionKey("tgt_orders:spec_1"),
    );
    expect(stableOptionKey("tgt_orders:spec_2")).not.toBe(
      stableOptionKey("tgt_orders:spec_1"),
    );
  });

  it("uses a same-origin first-paint print placeholder", () => {
    expect(PRINT_PLACEHOLDER_URL).toBe("/api/public/print-placeholder");
  });
  it("uses the configured ready default", () => {
    expect(
      chooseDefault([
        { id: "offline", eligible: false },
        { id: "ready", eligible: true, isDefault: true },
      ]),
    ).toMatchObject({ id: "ready" });
  });

  it("defaults documents to the merchant choice, then packing slip and invoice", () => {
    const documents = [
      { id: "invoice", name: "Invoice", kind: "invoice" },
      { id: "packing", name: "Packing Slip", kind: "packing_slip" },
    ];
    expect(chooseDefaultDocument(documents)?.id).toBe("packing");
    expect(
      chooseDefaultDocument([
        { id: "invoice", name: "Facture", kind: "invoice" },
        { id: "packing", name: "Bon de livraison", kind: "packing_slip" },
      ])?.id,
    ).toBe("packing");
    expect(
      chooseDefaultDocument([
        ...documents,
        { id: "chosen", name: "Warehouse", isDefault: true },
      ])?.id,
    ).toBe("chosen");
    expect(chooseDefaultDocument([documents[0]])?.id).toBe("invoice");
  });

  it("selects each document's own compatible target when documents change", () => {
    const targets = [
      { id: "receipt", eligible: true },
      { id: "label", eligible: true, isDefault: true },
    ];
    expect(
      targetForDocument(
        {
          designTargetId: "receipt",
          designSpecificationRevision: "spec_receipt_1",
          targetBindingStatus: "ready",
          compatibilityKnown: true,
          compatibleTargetIds: ["receipt"],
        },
        targets,
      )?.id,
    ).toBe("receipt");
    expect(
      targetForDocument(
        {
          designTargetId: "label",
          designSpecificationRevision: "spec_label_1",
          targetBindingStatus: "ready",
          compatibilityKnown: true,
          compatibleTargetIds: ["label"],
        },
        targets,
      )?.id,
    ).toBe("label");
    expect(
      targetForDocument(
        {
          designTargetId: null,
          designSpecificationRevision: null,
          targetBindingStatus: "unbound",
          compatibilityKnown: false,
          compatibleTargetIds: ["receipt", "label"],
        },
        targets,
      ),
    ).toBeUndefined();
  });

  it("fails closed unless the published target and revision are both ready", () => {
    expect(
      canUsePublishedBinding({
        targetBindingStatus: "ready",
        designTargetId: "tgt_orders",
        designSpecificationRevision: "spec_orders_4",
      }),
    ).toBe(true);
    for (const document of [
      {
        targetBindingStatus: "unbound",
        designTargetId: null,
        designSpecificationRevision: null,
      },
      {
        targetBindingStatus: "revision_changed",
        designTargetId: "tgt_orders",
        designSpecificationRevision: "spec_orders_4",
      },
      {
        targetBindingStatus: "ready",
        designTargetId: "tgt_orders",
        designSpecificationRevision: null,
      },
    ])
      expect(canUsePublishedBinding(document)).toBe(false);
  });

  it("shows a useful backend error", () => {
    expect(messageForLoadError(new Error("Connect Piqae first"))).toBe(
      "Connect Piqae first",
    );
  });

  it("aborts a stalled configuration request", async () => {
    vi.useFakeTimers();
    const pending = loadWithTimeout(
      (signal) =>
        new Promise((_, reject) => {
          signal.addEventListener("abort", () =>
            reject(new DOMException("Timed out", "AbortError")),
          );
        }),
      50,
    );
    const assertion = expect(pending).rejects.toMatchObject({
      name: "AbortError",
    });
    await vi.advanceTimersByTimeAsync(50);
    await assertion;
    vi.useRealTimers();
  });

  it("submits target routing for every policy instead of trusting one printer preflight", () => {
    const destination = {
      eligible: true,
    };
    expect(canUseDestinationForPolicy(destination, "automatic")).toBe(true);
    expect(canUseDestinationForPolicy(destination, "prefer_node")).toBe(true);
    expect(canUseDestinationForPolicy(destination, "require_node")).toBe(true);
    destination.eligible = false;
    expect(canUseDestinationForPolicy(destination, "require_node")).toBe(false);
  });

  it("explains fallback without claiming an unverified node is ready", () => {
    expect(renderPolicySummary("prefer_node")).toContain("falls back");
    expect(renderPolicySummary("require_node")).toContain(
      "checks every compatible target binding",
    );
  });
});
