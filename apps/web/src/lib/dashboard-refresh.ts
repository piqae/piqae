export function isTransientDashboardLoadError(error: unknown): boolean {
  if (!(error instanceof TypeError)) return false;
  return /^(?:Load failed|Failed to fetch|NetworkError)/i.test(error.message.trim());
}

/**
 * SvelteKit invalidation performs a client-side data fetch. Browsers reject it
 * when connectivity changes or a deployment replaces the current client. That
 * must not become an unhandled promise rejection; unexpected failures remain
 * observable through the supplied reporter.
 */
export async function settleDashboardRefresh(
  refresh: Promise<void>,
  report: (error: unknown) => void = (error) =>
    console.error('Dashboard refresh failed', {
      kind: error instanceof Error ? error.name : typeof error
    })
): Promise<void> {
  try {
    await refresh;
  } catch (error) {
    if (!isTransientDashboardLoadError(error)) report(error);
  }
}
