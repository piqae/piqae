const RETURN_TO_ORIGIN = 'https://piqae.invalid';

export function safeReturnTo(value: string | null, fallback = '/dashboard'): string {
  if (!value?.startsWith('/')) return fallback;
  try {
    const target = new URL(value, RETURN_TO_ORIGIN);
    return target.origin === RETURN_TO_ORIGIN
      ? `${target.pathname}${target.search}${target.hash}`
      : fallback;
  } catch {
    return fallback;
  }
}
