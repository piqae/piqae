import type { Breadcrumb, Event } from "@sentry/react-router";

/**
 * Privacy boundary for Sentry in the Shopify app.
 *
 * This is a deliberate port of `apps/web/src/lib/observability/sentry.ts`.
 * The two applications are separate pnpm workspace packages with different
 * frameworks, module resolution, and Sentry SDKs (`@sentry/sveltekit` vs
 * `@sentry/react-router`), so importing across the app boundary would break
 * build isolation. The redaction rules are kept identical and then widened
 * for Shopify-specific merchant and buyer identifiers.
 */

const REDACTED = "[redacted]";
const REDACTED_SHOP = "[shop]";
const MAX_TEXT_LENGTH = 1_000;

/**
 * Keys whose values are never reportable. The first half mirrors the web app.
 * The Shopify half covers merchant identity (shop domain), buyer identity
 * (customer, names, addresses, contact details), free-text order notes, and
 * webhook/App Bridge authentication material.
 */
const SENSITIVE_KEY =
  /(?:auth(?:orization)?|cookie|token|secret|password|passcode|api[-_]?key|session|device[-_]?code|enrol(?:l)?ment|document(?:[-_]?url)?|email|phone|address|username|full[-_]?name|^shop(?:[-_]?(?:domain|name|id|origin))?$|myshopify|shopify[-_]?domain|customer|recipient|contact|first[-_]?name|last[-_]?name|display[-_]?name|company|street|^city$|^zip$|postal|postcode|province|note|hmac|signature|id[-_]?token)/i;

const EMAIL = /\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi;
const AUTH_VALUE = /\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+/gi;
const SECRET_PAIR =
  /(^|[?&"'\s])((?:access_token|refresh_token|id[-_]?token|session[-_]?token|token|secret|password|api[-_]?key|device[-_]?code|enrol(?:l)?ment|hmac|signature)[="' :]+)([^&"',\s]+)/gi;
const UUID =
  /\b[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b/gi;
const LONG_IDENTIFIER = /\b[A-Za-z0-9_-]{32,}\b/g;

/** `example-store.myshopify.com` names one merchant and is treated as identity. */
const SHOP_DOMAIN = /\b[a-z0-9][a-z0-9-]*\.myshopify\.com\b/gi;
/** `gid://shopify/Customer/12345` — keep the resource kind, drop the record. */
const SHOPIFY_GID = /gid:\/\/shopify\/([A-Za-z]+)\/[A-Za-z0-9_.-]+/gi;
/** Shopify Admin API access, custom-app, and storefront tokens. */
const SHOPIFY_TOKEN = /\bshp(?:at|ca|pa|ss|ua)_[A-Za-z0-9]{8,}\b/gi;
/** App Bridge session tokens and any other JWT that reaches a message body. */
const JWT = /\beyJ[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}\.[A-Za-z0-9_-]{6,}\b/g;
/** Shopify legacy REST identifiers (orders, customers, products) are numeric. */
const NUMERIC_IDENTIFIER = /\b\d{6,}\b/g;

function truncate(value: string): string {
  return value.length <= MAX_TEXT_LENGTH
    ? value
    : `${value.slice(0, MAX_TEXT_LENGTH)}…`;
}

export function redactSentryText(value: string): string {
  return truncate(
    value
      .replace(EMAIL, REDACTED)
      .replace(AUTH_VALUE, `$1 ${REDACTED}`)
      .replace(SECRET_PAIR, `$1$2${REDACTED}`)
      .replace(SHOP_DOMAIN, REDACTED_SHOP)
      .replace(SHOPIFY_GID, `gid://shopify/$1/${REDACTED}`)
      .replace(SHOPIFY_TOKEN, REDACTED)
      .replace(JWT, REDACTED),
  );
}

export function sanitizeSentryUrl(value: string): string {
  try {
    const absolute = /^[a-z][a-z\d+.-]*:\/\//i.test(value);
    const parsed = new URL(value, "https://piqae.invalid");
    const safePath = parsed.pathname
      .replace(UUID, ":id")
      .replace(LONG_IDENTIFIER, ":id")
      .replace(NUMERIC_IDENTIFIER, ":id")
      .replace(EMAIL, REDACTED);
    return redactSentryText(
      absolute ? `${parsed.protocol}//${parsed.host}${safePath}` : safePath,
    );
  } catch {
    return redactSentryText(value.split(/[?#]/, 1)[0] ?? "");
  }
}

function redactUnknown(value: unknown, key = "", depth = 0): unknown {
  if (SENSITIVE_KEY.test(key)) return REDACTED;
  if (depth > 4) return "[truncated]";
  if (typeof value === "string") {
    const normalizedKey = key.toLowerCase();
    return normalizedKey.includes("url") ||
      normalizedKey === "from" ||
      normalizedKey === "to"
      ? sanitizeSentryUrl(value)
      : redactSentryText(value);
  }
  if (Array.isArray(value)) {
    return value
      .slice(0, 50)
      .map((entry) => redactUnknown(entry, "", depth + 1));
  }
  if (!value || typeof value !== "object") return value;

  return Object.fromEntries(
    Object.entries(value)
      .slice(0, 100)
      .map(([entryKey, entryValue]) => [
        entryKey,
        redactUnknown(entryValue, entryKey, depth + 1),
      ]),
  );
}

export function sanitizeSentryEvent<T extends Event>(event: T): T {
  const sanitized: Event = {
    ...event,
    user: undefined,
    server_name: undefined,
    message: event.message ? redactSentryText(event.message) : event.message,
    transaction: event.transaction
      ? redactSentryText(event.transaction)
      : event.transaction,
    extra: redactUnknown(event.extra) as Event["extra"],
    contexts: redactUnknown(event.contexts) as Event["contexts"],
    tags: redactUnknown(event.tags) as Event["tags"],
    spans: event.spans?.map((span) => ({
      ...span,
      description: span.description
        ? redactSentryText(span.description)
        : span.description,
      data: redactUnknown(span.data) as typeof span.data,
    })),
  };

  if (event.request) {
    sanitized.request = {
      method: event.request.method,
      url: event.request.url ? sanitizeSentryUrl(event.request.url) : undefined,
    };
  }

  sanitized.exception = event.exception
    ? {
        ...event.exception,
        values: event.exception.values?.map((exception) => ({
          ...exception,
          value: exception.value
            ? redactSentryText(exception.value)
            : exception.value,
          stacktrace: exception.stacktrace
            ? {
                ...exception.stacktrace,
                frames: exception.stacktrace.frames?.map((frame) => ({
                  ...frame,
                  vars: undefined,
                })),
              }
            : exception.stacktrace,
        })),
      }
    : event.exception;

  sanitized.breadcrumbs = event.breadcrumbs
    ?.map((breadcrumb) => sanitizeSentryBreadcrumb(breadcrumb))
    .filter((breadcrumb): breadcrumb is Breadcrumb => breadcrumb !== null);

  return sanitized as T;
}

export function sanitizeSentryBreadcrumb(
  breadcrumb: Breadcrumb,
): Breadcrumb | null {
  if (
    breadcrumb.category === "console" ||
    breadcrumb.category?.startsWith("ui.") ||
    breadcrumb.type === "user"
  ) {
    return null;
  }

  return {
    ...breadcrumb,
    message: breadcrumb.message
      ? redactSentryText(breadcrumb.message)
      : breadcrumb.message,
    data: redactUnknown(breadcrumb.data) as Breadcrumb["data"],
  };
}

export function sentrySampleRate(value: string | undefined): number {
  if (!value?.trim()) return 0;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 && parsed <= 1 ? parsed : 0;
}

export function safeErrorKind(error: unknown): string {
  if (error instanceof Error && error.name.trim()) return error.name;
  return typeof error === "object" && error !== null
    ? "NonErrorObject"
    : typeof error;
}

export function publicErrorMessage(status: number, message: string): string {
  return status >= 500
    ? "An unexpected error occurred."
    : redactSentryText(message);
}

/**
 * Origin the browser SDK posts events to, so the document CSP can allow it
 * only when browser reporting is actually configured.
 */
export function sentryIngestOrigin(dsn: string | undefined): string | null {
  const normalized = dsn?.trim();
  if (!normalized) return null;
  try {
    const parsed = new URL(normalized);
    return parsed.protocol === "https:" || parsed.protocol === "http:"
      ? parsed.origin
      : null;
  } catch {
    return null;
  }
}

export type SentryRuntimeConfiguration = {
  dsn: string;
  environment?: string;
  release?: string;
  tracesSampleRate: number;
};

type RuntimeEnvironment = Record<string, string | undefined>;

function present(value: string | undefined): string | undefined {
  const normalized = value?.trim();
  return normalized ? normalized : undefined;
}

function resolve(
  environment: RuntimeEnvironment,
  prefix: "" | "PUBLIC_",
): SentryRuntimeConfiguration | null {
  const dsn = present(environment[`${prefix}SENTRY_DSN`]);
  // An unusable DSN must stay inert rather than half-initialize the SDK.
  if (!dsn || !sentryIngestOrigin(dsn)) return null;
  return {
    dsn,
    environment: present(environment[`${prefix}SENTRY_ENVIRONMENT`]),
    release: present(environment[`${prefix}SENTRY_RELEASE`]),
    tracesSampleRate: sentrySampleRate(
      environment[`${prefix}SENTRY_TRACES_SAMPLE_RATE`],
    ),
  };
}

/**
 * Server configuration. Uses the same variable names as the web app:
 * `SENTRY_DSN`, `SENTRY_ENVIRONMENT`, `SENTRY_RELEASE`,
 * `SENTRY_TRACES_SAMPLE_RATE`. Returns `null` when no DSN is configured, which
 * is the signal to leave Sentry completely uninitialized.
 */
export function resolveServerSentryConfiguration(
  environment: RuntimeEnvironment,
): SentryRuntimeConfiguration | null {
  return resolve(environment, "");
}

/**
 * Browser configuration. Browser reporting is a separate operator opt-in and
 * uses the `PUBLIC_`-prefixed names, exactly as the web app does, so a
 * server-only DSN is never serialized into an embedded Shopify Admin page.
 */
export function resolveBrowserSentryConfiguration(
  environment: RuntimeEnvironment,
): SentryRuntimeConfiguration | null {
  return resolve(environment, "PUBLIC_");
}

/**
 * The only Sentry variables that may be published to the browser. The
 * server-only `SENTRY_DSN` is deliberately absent.
 */
export const BROWSER_SENTRY_ENVIRONMENT_KEYS = [
  "PUBLIC_SENTRY_DSN",
  "PUBLIC_SENTRY_ENVIRONMENT",
  "PUBLIC_SENTRY_RELEASE",
  "PUBLIC_SENTRY_TRACES_SAMPLE_RATE",
] as const;

/** Browser settings to serialize into the document, or `null` when reporting is off. */
export function browserSentryEnvironment(
  environment: RuntimeEnvironment,
): Record<string, string> | null {
  const published: Record<string, string> = {};
  for (const key of BROWSER_SENTRY_ENVIRONMENT_KEYS) {
    const value = present(environment[key]);
    if (value) published[key] = value;
  }
  return resolveBrowserSentryConfiguration(published) ? published : null;
}

/** Inline bootstrap read by `app/observability/sentry.client.ts`. */
export function browserSentryBootstrapScript(
  published: Record<string, string> | null,
): string | null {
  if (!published) return null;
  return `window.__piqaeSentry=${JSON.stringify(published).replace(/</g, "\\u003c")};`;
}
