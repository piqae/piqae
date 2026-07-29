import type { Breadcrumb, Event } from '@sentry/sveltekit';

const REDACTED = '[redacted]';
const MAX_TEXT_LENGTH = 1_000;
const SENSITIVE_KEY =
  /(?:auth(?:orization)?|cookie|token|secret|password|passcode|api[-_]?key|session|device[-_]?code|enrol(?:l)?ment|document(?:[-_]?url)?|email|phone|address|username|full[-_]?name)/i;
const EMAIL = /\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi;
const AUTH_VALUE = /\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+/gi;
const SECRET_PAIR =
  /(^|[?&"'\s])((?:access_token|refresh_token|token|secret|password|api[-_]?key|device[-_]?code|enrol(?:l)?ment)[="' :]+)([^&"',\s]+)/gi;
const UUID = /\b[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}\b/gi;
const LONG_IDENTIFIER = /\b[A-Za-z0-9_-]{32,}\b/g;

function truncate(value: string): string {
  return value.length <= MAX_TEXT_LENGTH ? value : `${value.slice(0, MAX_TEXT_LENGTH)}…`;
}

export function redactSentryText(value: string): string {
  return truncate(
    value
      .replace(EMAIL, REDACTED)
      .replace(AUTH_VALUE, `$1 ${REDACTED}`)
      .replace(SECRET_PAIR, `$1$2${REDACTED}`)
  );
}

export function sanitizeSentryUrl(value: string): string {
  try {
    const absolute = /^[a-z][a-z\d+.-]*:\/\//i.test(value);
    const parsed = new URL(value, 'https://spool.invalid');
    const safePath = parsed.pathname
      .replace(UUID, ':id')
      .replace(LONG_IDENTIFIER, ':id')
      .replace(EMAIL, REDACTED);
    return redactSentryText(absolute ? `${parsed.protocol}//${parsed.host}${safePath}` : safePath);
  } catch {
    return redactSentryText(value.split(/[?#]/, 1)[0] ?? '');
  }
}

function redactUnknown(value: unknown, key = '', depth = 0): unknown {
  if (SENSITIVE_KEY.test(key)) return REDACTED;
  if (depth > 4) return '[truncated]';
  if (typeof value === 'string') {
    const normalizedKey = key.toLowerCase();
    return normalizedKey.includes('url') || normalizedKey === 'from' || normalizedKey === 'to'
      ? sanitizeSentryUrl(value)
      : redactSentryText(value);
  }
  if (Array.isArray(value)) {
    return value.slice(0, 50).map((entry) => redactUnknown(entry, '', depth + 1));
  }
  if (!value || typeof value !== 'object') return value;

  return Object.fromEntries(
    Object.entries(value)
      .slice(0, 100)
      .map(([entryKey, entryValue]) => [
        entryKey,
        redactUnknown(entryValue, entryKey, depth + 1)
      ])
  );
}

export function sanitizeSentryEvent<T extends Event>(event: T): T {
  const sanitized: Event = {
    ...event,
    user: undefined,
    server_name: undefined,
    message: event.message ? redactSentryText(event.message) : event.message,
    transaction: event.transaction ? redactSentryText(event.transaction) : event.transaction,
    extra: redactUnknown(event.extra) as Event['extra'],
    contexts: redactUnknown(event.contexts) as Event['contexts'],
    tags: redactUnknown(event.tags) as Event['tags'],
    spans: event.spans?.map((span) => ({
      ...span,
      data: redactUnknown(span.data) as typeof span.data
    }))
  };

  if (event.request) {
    sanitized.request = {
      method: event.request.method,
      url: event.request.url ? sanitizeSentryUrl(event.request.url) : undefined
    };
  }

  sanitized.exception = event.exception
    ? {
        ...event.exception,
        values: event.exception.values?.map((exception) => ({
          ...exception,
          value: exception.value ? redactSentryText(exception.value) : exception.value,
          stacktrace: exception.stacktrace
            ? {
                ...exception.stacktrace,
                frames: exception.stacktrace.frames?.map((frame) => ({
                  ...frame,
                  vars: undefined
                }))
              }
            : exception.stacktrace
        }))
      }
    : event.exception;

  sanitized.breadcrumbs = event.breadcrumbs
    ?.map((breadcrumb) => sanitizeSentryBreadcrumb(breadcrumb))
    .filter((breadcrumb): breadcrumb is Breadcrumb => breadcrumb !== null);

  return sanitized as T;
}

export function sanitizeSentryBreadcrumb(breadcrumb: Breadcrumb): Breadcrumb | null {
  if (
    breadcrumb.category === 'console' ||
    breadcrumb.category?.startsWith('ui.') ||
    breadcrumb.type === 'user'
  ) {
    return null;
  }

  return {
    ...breadcrumb,
    message: breadcrumb.message ? redactSentryText(breadcrumb.message) : breadcrumb.message,
    data: redactUnknown(breadcrumb.data) as Breadcrumb['data']
  };
}

export function sentrySampleRate(value: string | undefined): number {
  if (!value?.trim()) return 0;
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed >= 0 && parsed <= 1 ? parsed : 0;
}

export function safeErrorKind(error: unknown): string {
  if (error instanceof Error && error.name.trim()) return error.name;
  return typeof error === 'object' && error !== null ? 'NonErrorObject' : typeof error;
}

export function publicErrorMessage(status: number, message: string): string {
  return status >= 500 ? 'An unexpected error occurred.' : redactSentryText(message);
}
