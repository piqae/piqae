export interface NodeConnectFragment {
  enrolmentToken: string;
  controlPlaneUrl: string;
}

const ENROLMENT_TOKEN = /^piq_enr_[A-Za-z0-9_-]{32}$/;

function safeControlPlaneUrl(value: string): string | null {
  try {
    const url = new URL(value);
    const localHttp = url.protocol === 'http:' && ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
    if ((url.protocol !== 'https:' && !localHttp) || url.username || url.password || url.search || url.hash) return null;
    return url.toString().replace(/\/$/, '');
  } catch {
    return null;
  }
}

export function nativeNodeConnectUrl(enrolmentToken: string, controlPlaneUrl: string): string | null {
  if (!ENROLMENT_TOKEN.test(enrolmentToken)) return null;
  const origin = safeControlPlaneUrl(controlPlaneUrl);
  if (!origin) return null;
  return `piqae://connect#enrolment_token=${encodeURIComponent(enrolmentToken)}&control_plane_url=${encodeURIComponent(origin)}`;
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
  const controlPlaneUrl = parameters.get('control_plane_url');
  if (!token) return null;

  replaceUrl(`${location.pathname}${location.search}`);
  const origin = controlPlaneUrl ? safeControlPlaneUrl(controlPlaneUrl) : null;
  return ENROLMENT_TOKEN.test(token) && origin ? { enrolmentToken: token, controlPlaneUrl: origin } : null;
}
