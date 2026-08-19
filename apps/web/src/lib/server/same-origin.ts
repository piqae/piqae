/**
 * Require a browser mutation to originate from this deployment. SameSite
 * cookies are useful defence in depth, but are not a substitute for checking
 * the request origin at the server boundary.
 */
export function isSameOriginRequest(request: Request, url: URL): boolean {
  const origin = request.headers.get('origin');
  if (!origin) return false;
  try {
    return new URL(origin).origin === url.origin;
  } catch {
    return false;
  }
}
