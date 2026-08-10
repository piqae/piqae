import { describe, expect, it, vi } from "vitest";

import {
  chooseDefault,
  loadWithTimeout,
  messageForLoadError,
  newInteractionId,
  PRINT_PLACEHOLDER_URL,
} from "./AdminOrderPrintAction.jsx";

describe("admin print action state", () => {
  it("creates bounded request IDs without Web Crypto", () => {
    const first = newInteractionId(["gid://shopify/Order/1004"]);
    const second = newInteractionId(["gid://shopify/Order/1004"]);
    expect(first).toMatch(/Order-1004-[a-z0-9]+-[a-z0-9]+$/);
    expect(second).not.toBe(first);
    expect(first.length).toBeLessThanOrEqual(128);
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
});
