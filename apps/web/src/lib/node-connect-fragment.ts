export interface NodeConnectFragment {
  enrolmentToken: string;
}

const ENROLMENT_TOKEN = /^piq_enr_[A-Za-z0-9_-]{32}$/;

export function nativeNodeConnectUrl(enrolmentToken: string): string | null {
  if (!ENROLMENT_TOKEN.test(enrolmentToken)) return null;
  return `piqae://connect#enrolment_token=${encodeURIComponent(enrolmentToken)}`;
}

/**
 * Takes the one-time node capability out of the address bar without ever
 * sending it to the server or persisting it in web storage.
 */
export function consumeNodeConnectFragment(
  location: Pick<Location, 'hash' | 'pathname' | 'search'>,
  replaceUrl: (url: string) => void
): NodeConnectFragment | null {
  if (!location.hash.startsWith('#')) return null;
  const parameters = new URLSearchParams(location.hash.slice(1));
  const token = parameters.get('enrolment_token');
  if (!token) return null;

  replaceUrl(`${location.pathname}${location.search}`);
  return ENROLMENT_TOKEN.test(token) ? { enrolmentToken: token } : null;
}
