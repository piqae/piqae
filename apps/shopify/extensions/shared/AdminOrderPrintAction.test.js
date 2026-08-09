import { describe, expect, it, vi } from "vitest";

import {
  chooseDefault,
  loadWithTimeout,
  messageForLoadError,
} from "./AdminOrderPrintAction.jsx";

describe("admin print action state", () => {
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
