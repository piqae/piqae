import { describe, expect, it } from 'vitest';
import type { Breadcrumb, Event } from '@sentry/sveltekit';
import {
  publicErrorMessage,
  sanitizeSentryBreadcrumb,
  sanitizeSentryEvent,
  sanitizeSentryUrl,
  sentrySampleRate
} from './sentry';
import { resolveSentryBuildConfiguration } from './sentry-build';

describe('Sentry privacy boundary', () => {
  it('removes identity, request secrets, query strings, and local variables', () => {
    const event = sanitizeSentryEvent({
      user: { id: 'user-1', email: 'person@example.com' },
      server_name: 'private-host',
      request: {
        method: 'POST',
        url: 'https://app.example.com/jobs/123?token=not-safe',
        headers: { authorization: 'Bearer not-safe', cookie: 'session=not-safe' },
        data: { document_url: 'https://objects.example.com/file?signature=not-safe' }
      },
      extra: {
        workspace: 'safe-workspace',
        apiKey: 'not-safe',
        note: 'Contact person@example.com with Bearer not-safe'
      },
      spans: [
        {
          span_id: '0123456789abcdef',
          trace_id: '0123456789abcdef0123456789abcdef',
          start_timestamp: 1,
          timestamp: 2,
          op: 'http.client',
          data: {
            url: 'https://api.example.com/jobs?access_token=not-safe',
            authorization: 'Bearer not-safe'
          }
        }
      ],
      exception: {
        values: [
          {
            type: 'Error',
            value: 'access_token=not-safe for person@example.com',
            stacktrace: {
              frames: [{ filename: 'route.ts', vars: { access_token: 'not-safe' } }]
            }
          }
        ]
      }
    } satisfies Event);

    expect(event.user).toBeUndefined();
    expect(event.server_name).toBeUndefined();
    expect(event.request).toEqual({
      method: 'POST',
      url: 'https://app.example.com/jobs/123'
    });
    expect(event.extra).toEqual({
      workspace: 'safe-workspace',
      apiKey: '[redacted]',
      note: 'Contact [redacted] with Bearer [redacted]'
    });
    expect(event.exception?.values?.[0]?.value).not.toContain('not-safe');
    expect(event.exception?.values?.[0]?.value).not.toContain('person@example.com');
    expect(event.exception?.values?.[0]?.stacktrace?.frames?.[0]?.vars).toBeUndefined();
    expect(event.spans?.[0]?.data).toEqual({
      url: 'https://api.example.com/jobs',
      authorization: '[redacted]'
    });
  });

  it('drops interaction and console breadcrumbs and sanitizes navigation URLs', () => {
    expect(sanitizeSentryBreadcrumb({ category: 'ui.click', message: 'Print Alice' })).toBeNull();
    expect(
      sanitizeSentryBreadcrumb({ category: 'console', message: 'Bearer not-safe' })
    ).toBeNull();

    const breadcrumb = sanitizeSentryBreadcrumb({
      category: 'navigation',
      data: {
        from: '/jobs?token=not-safe',
        to: '/users/person@example.com'
      }
    } satisfies Breadcrumb);

    expect(breadcrumb?.data).toEqual({
      from: '/jobs',
      to: '/users/[redacted]'
    });
  });

  it('normalizes URLs and sampling configuration without leaking identifiers', () => {
    expect(
      sanitizeSentryUrl(
        'https://app.example.com/jobs/6ba7b810-9dad-11d1-80b4-00c04fd430c8?api_key=not-safe'
      )
    ).toBe('https://app.example.com/jobs/:id');
    expect(sentrySampleRate('0.05')).toBe(0.05);
    expect(sentrySampleRate('2')).toBe(0);
    expect(sentrySampleRate('invalid')).toBe(0);
    expect(publicErrorMessage(500, 'Database leaked person@example.com')).toBe(
      'An unexpected error occurred.'
    );
  });
});

describe('Sentry source-map release configuration', () => {
  it('leaves source maps disabled when upload credentials are absent', () => {
    expect(resolveSentryBuildConfiguration({ SENTRY_RELEASE: 'spool@abc123' })).toEqual({
      uploadSourceMaps: false
    });
  });

  it('requires one complete release-associated configuration', () => {
    expect(() =>
      resolveSentryBuildConfiguration({
        SENTRY_AUTH_TOKEN: 'secret-token',
        SENTRY_ORG: 'spool',
        SENTRY_PROJECT: 'web'
      })
    ).toThrow('missing SENTRY_RELEASE');
  });

  it('enables uploads only when token, target, and release are complete', () => {
    expect(
      resolveSentryBuildConfiguration({
        SENTRY_AUTH_TOKEN: 'secret-token',
        SENTRY_ORG: 'spool',
        SENTRY_PROJECT: 'web',
        SENTRY_RELEASE: 'spool-web@abc123'
      })
    ).toEqual({
      uploadSourceMaps: true,
      authToken: 'secret-token',
      organization: 'spool',
      project: 'web',
      release: 'spool-web@abc123'
    });
  });
});
