import { describe, expect, it } from 'vitest';
import { safeExternalHttpUrl } from './urls';

describe('marketing external URLs', () => {
  it('allows only absolute HTTP and HTTPS links', () => {
    expect(safeExternalHttpUrl('https://example.com/pricing')).toBe(
      'https://example.com/pricing'
    );
    expect(safeExternalHttpUrl('http://example.com/source')).toBe('http://example.com/source');
    expect(safeExternalHttpUrl('javascript:alert(1)')).toBeNull();
    expect(safeExternalHttpUrl('data:text/html,unsafe')).toBeNull();
    expect(safeExternalHttpUrl('/relative')).toBeNull();
  });
});
