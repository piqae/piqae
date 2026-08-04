import type { JobOptions } from './types.js';

const ENVELOPE_VERSION = 'piqae-encrypted-job-v3' as const;
const SUITE = 'ECDH-ES-P256+HKDF-SHA256+A256GCMKW+A256GCM' as const;
const RECIPIENT_ALGORITHM = 'ECDH-ES-P256+HKDF-SHA256+A256GCMKW' as const;

export interface EncryptedJobRecipient {
  /** Stable identifier for a dedicated encryption key. Never use a node signing key here. */
  key_id: string;
  algorithm: 'ECDH-P256-HKDF-SHA256';
  /** P-256 DER SubjectPublicKeyInfo encoded as unpadded base64url. */
  public_key_spki: string;
}

export interface EncryptedJobBinding {
  envelope_id: string;
  workspace_id: string;
  environment_id: string;
  content_type: 'pdf' | 'raw';
  printer_id: string;
  target_id: string;
  profile_revision: string;
  options: CanonicalJobOptions;
  deliveries: number;
  expires_at: string;
  raw_authorized: boolean;
}

export interface CanonicalJobOptions {
  bin: string | null; collate: boolean | null; color: boolean | null; copies: number | null;
  dpi: string | null; duplex: 'one-sided' | 'long-edge' | 'short-edge' | null;
  fit_to_page: boolean | null; media: string | null; nup: number | null;
  pages: string | null; paper: string | null; rotate: 0 | 90 | 180 | 270 | null;
  native_options: Record<string, string>;
}

export interface EncryptedJobEnvelope {
  version: typeof ENVELOPE_VERSION;
  suite: typeof SUITE;
  binding: EncryptedJobBinding;
  ciphertext_sha256: string;
  iv: string;
  ciphertext: string;
  recipients: Array<{
    key_id: string;
    algorithm: typeof RECIPIENT_ALGORITHM;
    /** Ephemeral P-256 public key as a 65-byte uncompressed SEC1 point. */
    ephemeral_public_key: string;
    /** Fresh 96-bit AES-GCM key-wrap IV. */
    key_wrap_iv: string;
    /** Fresh 256-bit HKDF salt. */
    hkdf_salt: string;
    /** 32-byte content key ciphertext followed by its 16-byte GCM tag. */
    encrypted_content_key: string;
  }>;
}

export type EncryptedJobManifest = Omit<EncryptedJobEnvelope, 'ciphertext'>;

export function encryptedJobCiphertext(envelope: EncryptedJobEnvelope): Uint8Array<ArrayBuffer> {
  return decodeBase64Url(envelope.ciphertext);
}

export function encryptedJobManifest(envelope: EncryptedJobEnvelope): EncryptedJobManifest {
  const { ciphertext: _, ...manifest } = envelope;
  return manifest;
}

export interface EncryptJobOptions extends Omit<EncryptedJobBinding, 'envelope_id' | 'options'> {
  options?: JobOptions;
  recipients: EncryptedJobRecipient[];
  crypto?: Crypto;
}

/**
 * Encrypts print bytes before upload using Web Crypto. This is an additive
 * preview envelope; callers must submit it only to an encrypted-job API that
 * understands this version, never to the plaintext PDF/RAW job endpoint.
 */
export async function encryptJobContent(
  plaintext: BufferSource,
  options: EncryptJobOptions
): Promise<EncryptedJobEnvelope> {
  const crypto = options.crypto ?? globalThis.crypto;
  if (!crypto?.subtle) throw new TypeError('Web Crypto is required');
  if (options.recipients.length === 0) throw new TypeError('At least one recipient is required');
  const keyIds = new Set<string>();
  for (const recipient of options.recipients) {
    if (!recipient.key_id || keyIds.has(recipient.key_id)) {
      throw new TypeError('Recipient key IDs must be non-empty and unique');
    }
    validateKeyId(recipient.key_id);
    keyIds.add(recipient.key_id);
    if (recipient.algorithm !== 'ECDH-P256-HKDF-SHA256') throw new TypeError('Unsupported recipient algorithm');
  }

  const binding: EncryptedJobBinding = {
    envelope_id: `env_${encodeBase64Url(crypto.getRandomValues(new Uint8Array(18)))}`,
    workspace_id: options.workspace_id,
    environment_id: options.environment_id,
    content_type: options.content_type,
    printer_id: options.printer_id,
    target_id: options.target_id,
    profile_revision: options.profile_revision,
    options: canonicalJobOptions(options.options),
    deliveries: options.deliveries,
    expires_at: options.expires_at,
    raw_authorized: options.raw_authorized
  };
  validateBinding(binding);
  const additionalData = encryptedJobAdditionalData(binding);
  const contentKey = await crypto.subtle.generateKey({ name: 'AES-GCM', length: 256 }, true, [
    'encrypt'
  ]);
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ciphertext = await crypto.subtle.encrypt(
    { name: 'AES-GCM', iv, additionalData, tagLength: 128 },
    contentKey,
    plaintext
  );
  const rawContentKey = await crypto.subtle.exportKey('raw', contentKey);
  const recipients = await Promise.all(
    options.recipients.map(async (recipient) => {
      const recipientPublicKey = await crypto.subtle.importKey(
        'spki',
        decodeBase64Url(recipient.public_key_spki),
        { name: 'ECDH', namedCurve: 'P-256' },
        false,
        []
      );
      const ephemeral = await crypto.subtle.generateKey(
        { name: 'ECDH', namedCurve: 'P-256' },
        false,
        ['deriveBits']
      );
      const sharedSecret = await crypto.subtle.deriveBits(
        { name: 'ECDH', public: recipientPublicKey },
        ephemeral.privateKey,
        256
      );
      const hkdfKey = await crypto.subtle.importKey('raw', sharedSecret, 'HKDF', false, ['deriveKey']);
      const hkdfSalt = crypto.getRandomValues(new Uint8Array(32));
      const wrapKey = await crypto.subtle.deriveKey(
        {
          name: 'HKDF',
          hash: 'SHA-256',
          salt: hkdfSalt,
          info: encryptedJobKeyWrapInfo(binding.envelope_id, recipient.key_id)
        },
        hkdfKey,
        { name: 'AES-GCM', length: 256 },
        false,
        ['encrypt']
      );
      const keyWrapIv = crypto.getRandomValues(new Uint8Array(12));
      const wrapped = await crypto.subtle.encrypt(
        {
          name: 'AES-GCM',
          iv: keyWrapIv,
          additionalData,
          tagLength: 128
        },
        wrapKey,
        rawContentKey
      );
      return {
        key_id: recipient.key_id,
        algorithm: RECIPIENT_ALGORITHM,
        ephemeral_public_key: encodeBase64Url(
          await crypto.subtle.exportKey('raw', ephemeral.publicKey)
        ),
        key_wrap_iv: encodeBase64Url(keyWrapIv),
        hkdf_salt: encodeBase64Url(hkdfSalt),
        encrypted_content_key: encodeBase64Url(wrapped)
      };
    })
  );
  const digest = await crypto.subtle.digest('SHA-256', ciphertext);
  return {
    version: ENVELOPE_VERSION,
    suite: SUITE,
    binding,
    ciphertext_sha256: encodeBase64Url(digest),
    iv: encodeBase64Url(iv),
    ciphertext: encodeBase64Url(ciphertext),
    recipients
  };
}

export async function verifyEncryptedJobEnvelope(
  envelope: EncryptedJobEnvelope,
  crypto: Crypto = globalThis.crypto
): Promise<boolean> {
  if (envelope.version !== ENVELOPE_VERSION || envelope.suite !== SUITE) return false;
  validateBinding(envelope.binding);
  if (envelope.recipients.length === 0 || new Set(envelope.recipients.map((r) => r.key_id)).size !== envelope.recipients.length) return false;
  if (!envelope.recipients.every((recipient) => {
    try {
      validateKeyId(recipient.key_id);
      return recipient.algorithm === RECIPIENT_ALGORITHM
        && decodeBase64Url(recipient.ephemeral_public_key).byteLength === 65
        && decodeBase64Url(recipient.ephemeral_public_key)[0] === 4
        && decodeBase64Url(recipient.key_wrap_iv).byteLength === 12
        && decodeBase64Url(recipient.hkdf_salt).byteLength === 32
        && decodeBase64Url(recipient.encrypted_content_key).byteLength === 48;
    } catch {
      return false;
    }
  })) return false;
  const ciphertext = decodeBase64Url(envelope.ciphertext);
  if (decodeBase64Url(envelope.iv).byteLength !== 12 || ciphertext.byteLength < 17) return false;
  const digest = await crypto.subtle.digest('SHA-256', ciphertext);
  return constantTimeEqual(encodeBase64Url(digest), envelope.ciphertext_sha256);
}

export function encryptedJobAdditionalData(binding: EncryptedJobBinding): Uint8Array<ArrayBuffer> {
  validateBinding(binding);
  return copyBytes(new TextEncoder().encode(canonicalBinding(binding)));
}

/** Exact HKDF info bytes shared by browser and node implementations. */
export function encryptedJobKeyWrapInfo(
  envelopeId: string,
  keyId: string
): Uint8Array<ArrayBuffer> {
  validateKeyId(keyId);
  if (!envelopeId.startsWith('env_') || envelopeId.includes('\0')) {
    throw new TypeError('Envelope ID is invalid');
  }
  return copyBytes(
    new TextEncoder().encode(`piqae-content-key-wrap-v3\0${envelopeId}\0${keyId}`)
  );
}

function canonicalBinding(binding: EncryptedJobBinding): string {
  return JSON.stringify({
    envelope_id: binding.envelope_id,
    workspace_id: binding.workspace_id,
    environment_id: binding.environment_id,
    content_type: binding.content_type,
    printer_id: binding.printer_id,
    target_id: binding.target_id,
    profile_revision: binding.profile_revision,
    options: binding.options,
    deliveries: binding.deliveries,
    expires_at: binding.expires_at,
    raw_authorized: binding.raw_authorized
  });
}

export function canonicalJobOptions(options: JobOptions = {}): CanonicalJobOptions {
  const encoder = new TextEncoder();
  const native_options = Object.fromEntries(
    Object.entries(options.native_options ?? {}).sort(([a], [b]) => {
      const left = encoder.encode(a);
      const right = encoder.encode(b);
      const length = Math.min(left.length, right.length);
      for (let index = 0; index < length; index += 1) {
        if (left[index] !== right[index]) return left[index]! - right[index]!;
      }
      return left.length - right.length;
    })
  );
  return {
    bin: options.bin ?? null, collate: options.collate ?? null, color: options.color ?? null,
    copies: options.copies ?? null, dpi: options.dpi ?? null, duplex: options.duplex ?? null,
    fit_to_page: options.fit_to_page ?? null, media: options.media ?? null, nup: options.nup ?? null,
    pages: options.pages ?? null, paper: options.paper ?? null, rotate: options.rotate ?? null,
    native_options
  };
}

function validateBinding(binding: EncryptedJobBinding): void {
  if (binding.content_type !== 'pdf' && binding.content_type !== 'raw') throw new TypeError('Invalid content type');
  if (!binding.envelope_id.startsWith('env_') || !binding.workspace_id || !binding.environment_id || !binding.printer_id || !binding.target_id || !binding.profile_revision) throw new TypeError('Envelope tenant, destination, and profile binding are required');
  if (!Number.isInteger(binding.deliveries) || binding.deliveries < 1 || binding.deliveries > 100) throw new TypeError('Deliveries are invalid');
  if (binding.raw_authorized !== (binding.content_type === 'raw')) throw new TypeError('RAW authorization binding is invalid');
  const expiry = Date.parse(binding.expires_at);
  if (!Number.isFinite(expiry)) throw new TypeError('Expiry must be an RFC 3339 timestamp');
}

function validateKeyId(keyId: string): void {
  if (!keyId || keyId.length > 255 || keyId.includes('\0')) {
    throw new TypeError('Recipient key ID is invalid');
  }
}

function encodeBase64Url(value: ArrayBuffer | Uint8Array): string {
  const bytes = value instanceof Uint8Array ? value : new Uint8Array(value);
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
}

function decodeBase64Url(value: string): Uint8Array<ArrayBuffer> {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) throw new TypeError('Invalid base64url value');
  const base64 = value.replaceAll('-', '+').replaceAll('_', '/').padEnd(Math.ceil(value.length / 4) * 4, '=');
  const binary = atob(base64);
  const decoded = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) decoded[index] = binary.charCodeAt(index);
  return decoded;
}

function copyBytes(value: Uint8Array): Uint8Array<ArrayBuffer> {
  const copy = new Uint8Array(value.byteLength);
  copy.set(value);
  return copy;
}

function constantTimeEqual(left: string, right: string): boolean {
  if (left.length !== right.length) return false;
  let difference = 0;
  for (let index = 0; index < left.length; index += 1) difference |= left.charCodeAt(index) ^ right.charCodeAt(index);
  return difference === 0;
}
