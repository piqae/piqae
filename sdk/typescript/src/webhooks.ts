export interface VerifyWebhookOptions {
  /** Maximum accepted clock skew. Set to false only for deterministic replay tooling. */
  toleranceSeconds?: number | false;
  now?: Date | number;
}

export interface PiqaeWebhookHeaders {
  'piqae-signature'?: string | null;
  'piqae-timestamp'?: string | null;
}

/**
 * Verify a Piqae webhook against its exact, unparsed request body.
 *
 * The helper is Web Crypto based and works in Node 20+, browsers and modern
 * serverless runtimes. Never JSON.parse/stringify the body before verifying it.
 */
export async function verifyWebhookSignature(
  secret: string,
  body: string | Uint8Array,
  headers: Headers | PiqaeWebhookHeaders,
  options: VerifyWebhookOptions = {}
): Promise<boolean> {
  const signatureHeader = header(headers, 'piqae-signature');
  const timestampHeader = header(headers, 'piqae-timestamp');
  const timestamp = Number(timestampHeader);
  if (!signatureHeader || !timestampHeader || !Number.isSafeInteger(timestamp)) return false;

  const supplied = signatureHeader
    .split(',')
    .map((part) => part.trim())
    .find((part) => part.startsWith('v1='))
    ?.slice(3);
  if (!supplied) return false;

  const tolerance = options.toleranceSeconds === undefined ? 300 : options.toleranceSeconds;
  const now = options.now instanceof Date ? options.now.getTime() : (options.now ?? Date.now());
  if (tolerance !== false && Math.abs(Math.floor(now / 1000) - timestamp) > tolerance) return false;

  const encoder = new TextEncoder();
  const rawBody = typeof body === 'string' ? encoder.encode(body) : body;
  const prefix = encoder.encode(`${timestamp}.`);
  const signed = new Uint8Array(prefix.length + rawBody.length);
  signed.set(prefix);
  signed.set(rawBody, prefix.length);
  const key = await crypto.subtle.importKey(
    'raw',
    encoder.encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['verify']
  );
  let bytes: Uint8Array;
  try {
    bytes = Uint8Array.from(atob(supplied), (character) => character.charCodeAt(0));
  } catch {
    return false;
  }
  return crypto.subtle.verify('HMAC', key, bytes, signed);
}

function header(headers: Headers | PiqaeWebhookHeaders, name: keyof PiqaeWebhookHeaders) {
  return headers instanceof Headers ? headers.get(name) : headers[name];
}
