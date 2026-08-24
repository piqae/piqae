import { describe, expect, it } from "vitest";
import { getClient } from "@sentry/react-router";
import {
  captureServerException,
  serverSentryEnabled,
} from "../app/observability/sentry.server";
import { initializeBrowserSentry } from "../app/observability/sentry.client";

// `tests/setup.ts` never sets SENTRY_DSN, so importing the server observability
// module must not initialize a Sentry client or create a transport.
describe("Sentry server runtime without a DSN", () => {
  it("never initializes the SDK", () => {
    expect(process.env.SENTRY_DSN).toBeUndefined();
    expect(serverSentryEnabled).toBe(false);
    expect(getClient()).toBeUndefined();
  });

  it("discards captured exceptions", () => {
    expect(() =>
      captureServerException(new Error("boom"), "test"),
    ).not.toThrow();
    expect(getClient()).toBeUndefined();
  });
});

describe("Sentry browser runtime without a DSN", () => {
  it("never loads or initializes the browser SDK", async () => {
    await expect(initializeBrowserSentry()).resolves.toBeUndefined();
    expect(getClient()).toBeUndefined();
  });
});
