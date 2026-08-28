import { describe, expect, it, vi } from "vitest";

import {
  chooseDefault,
  loadWithTimeout,
  messageForLoadError,
  newInteractionId,
  stableOptionKey,
  PRINT_PLACEHOLDER_URL,
  canUseDestinationForPolicy,
  renderPolicySummary,
  nodeReadinessMessage,
  nodeFallbackWarning,
  targetForDocument,
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

  it("selects each document's own compatible target when documents change", () => {
    const targets = [
      { id: "receipt", eligible: true },
      { id: "label", eligible: true, isDefault: true },
    ];
    expect(
      targetForDocument(
        {
          designTargetId: "receipt",
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
          compatibilityKnown: true,
          compatibleTargetIds: ["label"],
        },
        targets,
      )?.id,
    ).toBe("label");
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

  it("fails closed when node rendering is required", () => {
    const destination = {
      eligible: true,
      nodeRendering: { supported: true, ready: false },
    };
    expect(canUseDestinationForPolicy(destination, "automatic")).toBe(true);
    expect(canUseDestinationForPolicy(destination, "prefer_node")).toBe(true);
    expect(canUseDestinationForPolicy(destination, "require_node")).toBe(false);
    destination.nodeRendering.ready = true;
    expect(canUseDestinationForPolicy(destination, "require_node")).toBe(true);
  });

  it("explains fallback without claiming an unverified node is ready", () => {
    expect(renderPolicySummary("prefer_node")).toContain("falls back");
    expect(renderPolicySummary("require_node")).toContain("stays blocked");
  });

  it("turns bounded cache readiness into merchant-facing status", () => {
    expect(
      nodeReadinessMessage({
        ready: false,
        missing_resources: ["a".repeat(64), "b".repeat(64)],
      }),
    ).toBe("Warming 2 required resources");
    expect(
      nodeReadinessMessage({
        ready: false,
        reason: "renderer_abi_unavailable",
        missing_resources: [],
      }),
    ).toContain("compatible document renderer");
  });

  it("warns while safely falling back for an older node renderer", () => {
    const olderNode = {
      ready: false,
      reason: "renderer_abi_unavailable",
      missing_resources: [],
    };
    expect(nodeFallbackWarning(olderNode, "automatic")).toContain(
      "continue using the exact cloud-rendered preview PDF",
    );
    expect(nodeFallbackWarning(olderNode, "prefer_node")).toContain(
      "latest Piqae document renderer",
    );
    expect(nodeFallbackWarning(olderNode, "require_node")).toBeNull();
    expect(
      nodeFallbackWarning({ ...olderNode, ready: true }, "automatic"),
    ).toBeNull();
  });
});
