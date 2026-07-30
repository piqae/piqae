import { describe, expect, it } from 'vitest';
import {
  licensedFontObjectKey,
  publishedLicensedFont,
  readBoundedFontStream
} from './licensed-font-origin';

describe('licensed font origin', () => {
  it('maps only purchased Exact WOFF2 assets', () => {
    expect(licensedFontObjectKey('exact-regular.woff2')).toBe(
      'webfonts/exact/exact-regular.woff2'
    );
    expect(licensedFontObjectKey('exact-bold.woff2')).toBe(
      'webfonts/exact/exact-bold.woff2'
    );
    expect(licensedFontObjectKey('exact-regular.woff')).toBeNull();
    expect(licensedFontObjectKey('../exact-regular.woff2')).toBeNull();
    expect(licensedFontObjectKey('unlicensed.woff2')).toBeNull();
  });

  it('fails closed when the private release origin is unavailable', async () => {
    await expect(publishedLicensedFont('exact-regular.woff2', {})).resolves.toBeNull();
  });

  it('cancels an oversized streamed font before reading the remaining body', async () => {
    let cancelled = false;
    let pulls = 0;
    const stream = new ReadableStream<Uint8Array>({
      pull(controller) {
        pulls += 1;
        controller.enqueue(new Uint8Array(40));
      },
      cancel() {
        cancelled = true;
      }
    });
    await expect(readBoundedFontStream(stream, 64)).resolves.toBeNull();
    expect(cancelled).toBe(true);
    expect(pulls).toBe(2);
  });
});
