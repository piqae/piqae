const WEB_HOSTS = new Set(['piqae.com', 'app.piqae.com']);
const API_HOSTS = new Set(['api.piqae.com', 'sync.piqae.com']);

const APP_PATH_PREFIXES = ['/dashboard', '/login', '/auth/', '/pair'];

function canonicalHost(hostname) {
  return hostname.toLowerCase().replace(/\.$/, '');
}

function redirect(destination) {
  return { kind: 'redirect', destination };
}

function proxy(origin, pathname) {
  return { kind: 'proxy', origin, pathname };
}

export function routeRequest(input) {
  const url = input instanceof URL ? input : new URL(input);
  const host = canonicalHost(url.hostname);

  if (host === 'www.piqae.com') {
    return redirect(new URL(`${url.pathname}${url.search}`, 'https://piqae.com').toString());
  }

  if (host === 'connect.piqae.com') {
    const path = url.pathname === '/' ? '/pair' : url.pathname;
    return redirect(new URL(`${path}${url.search}`, 'https://app.piqae.com').toString());
  }

  if (host === 'docs.piqae.com') {
    const path =
      url.pathname === '/'
        ? '/docs'
        : url.pathname.startsWith('/docs')
          ? url.pathname
          : `/docs${url.pathname}`;
    return redirect(new URL(`${path}${url.search}`, 'https://piqae.com').toString());
  }

  if (host === 'downloads.piqae.com') {
    const path =
      url.pathname === '/'
        ? '/downloads'
        : url.pathname.startsWith('/downloads')
          ? url.pathname
          : `/downloads${url.pathname}`;
    return redirect(new URL(`${path}${url.search}`, 'https://piqae.com').toString());
  }

  if (host === 'piqae.com' && APP_PATH_PREFIXES.some((prefix) => url.pathname.startsWith(prefix))) {
    return redirect(new URL(`${url.pathname}${url.search}`, 'https://app.piqae.com').toString());
  }

  if (host === 'app.piqae.com' && url.pathname === '/') {
    return redirect(new URL(`/dashboard${url.search}`, 'https://app.piqae.com').toString());
  }

  if (WEB_HOSTS.has(host)) {
    return proxy('web', url.pathname);
  }

  if (API_HOSTS.has(host)) {
    return proxy('api', url.pathname);
  }

  return { kind: 'reject' };
}

function originUrl(value, name) {
  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    throw new Error(`${name} must be an absolute URL`);
  }

  if (parsed.protocol !== 'https:' || parsed.username || parsed.password || parsed.search || parsed.hash) {
    throw new Error(`${name} must be a credential-free HTTPS origin`);
  }

  return parsed;
}

function upstreamRequest(request, target, originalHost) {
  const headers = new Headers(request.headers);
  headers.delete('x-piqae-original-host');
  headers.delete('x-forwarded-host');
  headers.delete('x-forwarded-proto');
  headers.set('x-piqae-original-host', originalHost);
  headers.set('x-forwarded-host', originalHost);
  headers.set('x-forwarded-proto', 'https');

  return new Request(target, {
    method: request.method,
    headers,
    body: request.body,
    redirect: 'manual'
  });
}

export default {
  async fetch(request, env) {
    const incoming = new URL(request.url);
    const host = canonicalHost(incoming.hostname);
    const route = routeRequest(incoming);

    if (incoming.pathname === '/.well-known/piqae-edge-health') {
      return Response.json(
        { status: 'ok', service: 'piqae-edge-router' },
        { headers: { 'cache-control': 'no-store' } }
      );
    }

    if (route.kind === 'redirect') {
      return new Response(null, {
        status: 308,
        headers: {
          location: route.destination,
          'cache-control': 'public, max-age=300'
        }
      });
    }

    if (route.kind === 'reject') {
      return new Response('Unknown Piqae host', {
        status: 421,
        headers: { 'cache-control': 'no-store' }
      });
    }

    const base =
      route.origin === 'api'
        ? originUrl(env.API_ORIGIN, 'API_ORIGIN')
        : originUrl(env.WEB_ORIGIN, 'WEB_ORIGIN');
    const target = new URL(request.url);
    target.protocol = base.protocol;
    target.hostname = base.hostname;
    target.port = base.port;
    target.pathname = route.pathname;

    const upstream = upstreamRequest(request, target, host);
    if (route.origin === 'api') {
      return fetch(upstream, { cache: 'no-store', redirect: 'manual' });
    }
    return fetch(upstream, { redirect: 'manual' });
  }
};
