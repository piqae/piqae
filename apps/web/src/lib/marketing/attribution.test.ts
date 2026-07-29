import { describe, expect, it } from 'vitest';
import { buildAttribution } from './attribution';

describe('marketing attribution', () => {
  it('allow-lists plan and strips unsafe campaign characters', () => {
    const result = buildAttribution(
      new URL('https://example.test/start?plan=pro&utm_campaign=%3Cscript%3E&source=hero')
    );
    expect(result.plan).toBe('pro');
    expect(result.source).toBe('hero');
    expect(result.lastTouch.utm_campaign).toBe('script');
  });

  it('retains the original first touch', () => {
    const first = buildAttribution(new URL('https://example.test/start?utm_source=search'));
    const second = buildAttribution(new URL('https://example.test/start?utm_source=docs'), first);
    expect(second.firstTouch.utm_source).toBe('search');
    expect(second.lastTouch.utm_source).toBe('docs');
  });

  it('stores only the referrer origin and path', () => {
    const result = buildAttribution(
      new URL('https://example.test/start'),
      undefined,
      'https://search.example/results?q=customer-name'
    );
    expect(result.lastTouch.referrer).toBe('https://search.example/results');
  });
});
