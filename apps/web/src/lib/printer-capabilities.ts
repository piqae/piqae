/**
 * CUPS/IPP drivers commonly report zero when a numeric capability has no
 * advertised upper bound. Keep the UI usable while still respecting a
 * positive driver-provided limit.
 */
export function copyLimit(reported: number | null | undefined): number {
  return typeof reported === 'number' && Number.isFinite(reported) && reported > 0
    ? Math.floor(reported)
    : 99;
}
