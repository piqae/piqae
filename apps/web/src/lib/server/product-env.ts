/**
 * Reads the canonical Piqae variable and falls back to its pre-rebrand name.
 *
 * The fallback is intentionally one-way: new deployments document and emit
 * only Piqae names, while existing installations keep working through V1.
 */
export function productEnvironmentValue(
  environment: Record<string, string | undefined>,
  key: string
): string | undefined {
  const canonical = environment[key];
  if (canonical !== undefined) return canonical;

  const legacyKey = key.startsWith('PUBLIC_PIQAE_')
    ? key.replace(/^PUBLIC_PIQAE_/, 'PUBLIC_SPOOL_')
    : key.startsWith('PIQAE_')
      ? key.replace(/^PIQAE_/, 'SPOOL_')
      : null;
  return legacyKey ? environment[legacyKey] : undefined;
}
