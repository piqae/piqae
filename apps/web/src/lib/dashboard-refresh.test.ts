import { describe, expect, it, vi } from 'vitest';
import { isTransientDashboardLoadError, settleDashboardRefresh } from './dashboard-refresh';

describe('dashboard live refresh', () => {
  it('recognises browser network failures without classifying application errors as transient', () => {
    expect(isTransientDashboardLoadError(new TypeError('Load failed'))).toBe(true);
    expect(isTransientDashboardLoadError(new TypeError('Failed to fetch'))).toBe(true);
    expect(isTransientDashboardLoadError(new Error('Load failed'))).toBe(false);
    expect(isTransientDashboardLoadError(new TypeError('Response parser failed'))).toBe(false);
  });

  it('settles transient fetch rejection and reports unexpected invalidation failures', async () => {
    const report = vi.fn();
    await expect(
      settleDashboardRefresh(Promise.reject(new TypeError('Load failed')), report)
    ).resolves.toBeUndefined();
    expect(report).not.toHaveBeenCalled();

    const unexpected = new Error('Invalid dashboard payload');
    await expect(settleDashboardRefresh(Promise.reject(unexpected), report)).resolves.toBeUndefined();
    expect(report).toHaveBeenCalledWith(unexpected);
  });
});
