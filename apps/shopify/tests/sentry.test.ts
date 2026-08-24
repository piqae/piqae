import { describe, expect, it } from "vitest";
import type { Breadcrumb, Event } from "@sentry/react-router";
import {
  browserSentryBootstrapScript,
  browserSentryEnvironment,
  publicErrorMessage,
  redactSentryText,
  resolveBrowserSentryConfiguration,
  resolveServerSentryConfiguration,
  safeErrorKind,
  sanitizeSentryBreadcrumb,
  sanitizeSentryEvent,
  sanitizeSentryUrl,
  sentryIngestOrigin,
  sentrySampleRate,
} from "../app/observability/sentry";

const SHOP = "demo-store.myshopify.com";
const ADMIN_TOKEN = "shpat_EXAMPLEnotARealTokenZZ";
// Assembled at runtime so no JWT-shaped literal sits in the file for secret
// scanners to flag. It still matches the JWT pattern the redactor looks for.
const SESSION_TOKEN = ["eyJ", "EXAMPLEheader", "EXAMPLEpayload", "EXAMPLEsig"]
  .join(".")
  .replace(".", "");

describe("Sentry privacy boundary", () => {
  it("removes identity, request secrets, query strings, and local variables", () => {
    const event = sanitizeSentryEvent({
      user: { id: "user-1", email: "person@example.com" },
      server_name: "private-host",
      request: {
        method: "POST",
        url: "https://apps.piqae.example/app/jobs/123?token=not-safe",
        headers: {
          authorization: "Bearer not-safe",
          cookie: "session=not-safe",
        },
        data: {
          document_url: "https://objects.example.com/file?signature=not-safe",
        },
      },
      extra: {
        workspace: "safe-workspace",
        apiKey: "not-safe",
        note: "Contact person@example.com with Bearer not-safe",
      },
      spans: [
        {
          span_id: "0123456789abcdef",
          trace_id: "0123456789abcdef0123456789abcdef",
          start_timestamp: 1,
          timestamp: 2,
          op: "http.client",
          description: "POST https://api.example.com/jobs?token=not-safe",
          data: {
            url: "https://api.example.com/jobs?access_token=not-safe",
            authorization: "Bearer not-safe",
          },
        },
      ],
      exception: {
        values: [
          {
            type: "Error",
            value: "access_token=not-safe for person@example.com",
            stacktrace: {
              frames: [
                { filename: "route.ts", vars: { access_token: "not-safe" } },
              ],
            },
          },
        ],
      },
    } satisfies Event);

    expect(event.user).toBeUndefined();
    expect(event.server_name).toBeUndefined();
    expect(event.request).toEqual({
      method: "POST",
      url: "https://apps.piqae.example/app/jobs/123",
    });
    expect(event.extra).toEqual({
      workspace: "safe-workspace",
      apiKey: "[redacted]",
      note: "[redacted]",
    });
    expect(event.exception?.values?.[0]?.value).toBe(
      "access_token=[redacted] for [redacted]",
    );
    expect(
      event.exception?.values?.[0]?.stacktrace?.frames?.[0]?.vars,
    ).toBeUndefined();
    expect(event.spans?.[0]?.data).toEqual({
      url: "https://api.example.com/jobs",
      authorization: "[redacted]",
    });
    expect(event.spans?.[0]?.description).not.toContain("not-safe");
  });

  it("drops interaction and console breadcrumbs and sanitizes navigation URLs", () => {
    expect(
      sanitizeSentryBreadcrumb({
        category: "ui.click",
        message: "Print Alice",
      }),
    ).toBeNull();
    expect(
      sanitizeSentryBreadcrumb({ category: "console", message: ADMIN_TOKEN }),
    ).toBeNull();

    const breadcrumb = sanitizeSentryBreadcrumb({
      category: "navigation",
      data: {
        from: `/app/orders?shop=${SHOP}`,
        to: "/app/customers/person@example.com",
      },
    } satisfies Breadcrumb);

    expect(breadcrumb?.data).toEqual({
      from: "/app/orders",
      to: "/app/customers/[redacted]",
    });
  });

  it("normalizes URLs and sampling configuration without leaking identifiers", () => {
    expect(
      sanitizeSentryUrl(
        "https://apps.piqae.example/app/jobs/6ba7b810-9dad-11d1-80b4-00c04fd430c8?api_key=not-safe",
      ),
    ).toBe("https://apps.piqae.example/app/jobs/:id");
    expect(sentrySampleRate("0.05")).toBe(0.05);
    expect(sentrySampleRate("2")).toBe(0);
    expect(sentrySampleRate("invalid")).toBe(0);
    expect(sentrySampleRate(undefined)).toBe(0);
    expect(publicErrorMessage(500, "Database leaked person@example.com")).toBe(
      "An unexpected error occurred.",
    );
    expect(publicErrorMessage(404, `No template for ${SHOP}`)).toBe(
      "No template for [shop]",
    );
    expect(safeErrorKind(new TypeError("boom"))).toBe("TypeError");
    expect(safeErrorKind({ nope: true })).toBe("NonErrorObject");
  });

  it("truncates long values", () => {
    const redacted = redactSentryText("a".repeat(1_500));
    expect(redacted).toHaveLength(1_001);
    expect(redacted.endsWith("…")).toBe(true);
  });
});

describe("Shopify merchant and buyer data", () => {
  it("strips shop domain, Shopify GIDs, access tokens, and session tokens from text", () => {
    expect(
      redactSentryText(
        `Order gid://shopify/Order/6123456789 for alice@example.com at ${SHOP} failed with ${ADMIN_TOKEN}`,
      ),
    ).toBe(
      "Order gid://shopify/Order/[redacted] for [redacted] at [shop] failed with [redacted]",
    );

    expect(redactSentryText(`App Bridge session ${SESSION_TOKEN}`)).toBe(
      "App Bridge session [redacted]",
    );

    expect(
      redactSentryText(
        `callback hmac=6c8f2d1e id_token=${SESSION_TOKEN} shop=${SHOP}`,
      ),
    ).toBe("callback hmac=[redacted] id_token=[redacted] shop=[shop]");

    expect(redactSentryText("customer gid://shopify/Customer/6123456789")).toBe(
      "customer gid://shopify/Customer/[redacted]",
    );
  });

  it("replaces the shop domain and legacy numeric identifiers in URLs", () => {
    expect(
      sanitizeSentryUrl(
        `https://${SHOP}/admin/orders/6123456789?access_token=${ADMIN_TOKEN}`,
      ),
    ).toBe("https://[shop]/admin/orders/:id");

    expect(
      sanitizeSentryUrl(
        `https://apps.piqae.example/app/templates/tmpl-42?shop=${SHOP}&id_token=${SESSION_TOKEN}`,
      ),
    ).toBe("https://apps.piqae.example/app/templates/tmpl-42");
  });

  it("scrubs merchant PII from a realistic Shopify webhook failure event", () => {
    const event = sanitizeSentryEvent({
      request: {
        method: "POST",
        url: `https://apps.piqae.example/webhooks?shop=${SHOP}&hmac=6c8f2d1e`,
        headers: { "x-shopify-hmac-sha256": "6c8f2d1e" },
      },
      tags: { shop: SHOP, route: "/webhooks" },
      contexts: {
        shopify: { shopDomain: SHOP, apiVersion: "2026-07" },
      },
      extra: {
        shop: SHOP,
        accessToken: ADMIN_TOKEN,
        orderName: "#1042",
        templateId: "tmpl-basic",
        note: "Leave with the neighbour",
        customer: {
          displayName: "Alice Merchant",
          email: "alice@example.com",
          phone: "+64 21 555 0100",
        },
        shippingAddress: {
          address1: "12 Queen Street",
          city: "Auckland",
          zip: "1010",
          country: "New Zealand",
        },
        summary: `Order gid://shopify/Order/6123456789 for ${SHOP}`,
      },
      exception: {
        values: [
          {
            type: "Error",
            value: `Render failed for alice@example.com at ${SHOP} (gid://shopify/Customer/6123456789)`,
          },
        ],
      },
    } satisfies Event);

    expect(event.request).toEqual({
      method: "POST",
      url: "https://apps.piqae.example/webhooks",
    });
    expect(event.tags).toEqual({ shop: "[redacted]", route: "/webhooks" });
    expect(event.contexts).toEqual({
      shopify: { shopDomain: "[redacted]", apiVersion: "2026-07" },
    });
    expect(event.extra).toEqual({
      shop: "[redacted]",
      accessToken: "[redacted]",
      orderName: "#1042",
      templateId: "tmpl-basic",
      note: "[redacted]",
      customer: "[redacted]",
      shippingAddress: "[redacted]",
      summary: "Order gid://shopify/Order/[redacted] for [shop]",
    });
    expect(event.exception?.values?.[0]?.value).toBe(
      "Render failed for [redacted] at [shop] (gid://shopify/Customer/[redacted])",
    );

    const serialized = JSON.stringify(event);
    for (const secret of [
      SHOP,
      ADMIN_TOKEN,
      "alice@example.com",
      "Alice Merchant",
      "12 Queen Street",
      "Auckland",
      "+64 21 555 0100",
      "6123456789",
    ]) {
      expect(serialized).not.toContain(secret);
    }
  });
});

describe("Sentry configuration gating", () => {
  it("stays inert when no DSN is configured", () => {
    expect(resolveServerSentryConfiguration({})).toBeNull();
    expect(resolveServerSentryConfiguration({ SENTRY_DSN: "   " })).toBeNull();
    expect(
      resolveServerSentryConfiguration({ SENTRY_DSN: "not-a-dsn" }),
    ).toBeNull();
    expect(resolveBrowserSentryConfiguration({})).toBeNull();
    expect(sentryIngestOrigin(undefined)).toBeNull();
  });

  it("never promotes the server DSN to the browser", () => {
    const environment = {
      SENTRY_DSN: "https://public@sentry.example/42",
      SENTRY_ENVIRONMENT: "production",
      SENTRY_RELEASE: "piqae-shopify@abc123",
      SENTRY_TRACES_SAMPLE_RATE: "0.1",
    };

    expect(resolveServerSentryConfiguration(environment)).toEqual({
      dsn: "https://public@sentry.example/42",
      environment: "production",
      release: "piqae-shopify@abc123",
      tracesSampleRate: 0.1,
    });
    expect(resolveBrowserSentryConfiguration(environment)).toBeNull();
  });

  it("resolves browser reporting from PUBLIC_ variables only", () => {
    expect(
      resolveBrowserSentryConfiguration({
        PUBLIC_SENTRY_DSN: "https://public@sentry.example/7",
        PUBLIC_SENTRY_ENVIRONMENT: "production",
        PUBLIC_SENTRY_TRACES_SAMPLE_RATE: "bogus",
      }),
    ).toEqual({
      dsn: "https://public@sentry.example/7",
      environment: "production",
      release: undefined,
      tracesSampleRate: 0,
    });
    expect(sentryIngestOrigin("https://public@sentry.example/7")).toBe(
      "https://sentry.example",
    );
  });
});

describe("Browser bootstrap payload", () => {
  it("publishes nothing when only the server DSN is configured", () => {
    expect(
      browserSentryEnvironment({
        SENTRY_DSN: "https://public@sentry.example/42",
        SENTRY_ENVIRONMENT: "production",
      }),
    ).toBeNull();
    expect(browserSentryBootstrapScript(null)).toBeNull();
  });

  it("publishes only PUBLIC_ variables and escapes script terminators", () => {
    const published = browserSentryEnvironment({
      SENTRY_DSN: "https://server-only@sentry.example/42",
      PUBLIC_SENTRY_DSN: "https://public@sentry.example/7",
      PUBLIC_SENTRY_ENVIRONMENT: "production</script>",
      PUBLIC_SENTRY_TRACES_SAMPLE_RATE: "0.1",
    });

    expect(published).toEqual({
      PUBLIC_SENTRY_DSN: "https://public@sentry.example/7",
      PUBLIC_SENTRY_ENVIRONMENT: "production</script>",
      PUBLIC_SENTRY_TRACES_SAMPLE_RATE: "0.1",
    });

    const script = browserSentryBootstrapScript(published);
    expect(script).not.toContain("server-only");
    expect(script).not.toContain("</script>");
    expect(script).toContain("\\u003c/script>");
  });
});
