import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import {
  ENCRYPTED_JOB_V3_RECIPIENT_ALGORITHM,
  ENCRYPTED_JOB_V3_SUITE,
  ENCRYPTED_JOB_V3_VERSION,
  canonicalJobOptions,
  encryptJobContent,
  encryptedJobAdditionalData,
  encryptedJobKeyWrapInfo,
  verifyEncryptedJobEnvelope
} from '../src/encrypted-jobs.js';

const base64url = (value: ArrayBuffer) =>
  Buffer.from(value).toString('base64url');

const decodeBase64 = (value: string) => Buffer.from(value, 'base64');

async function deterministicConformanceCrypto(): Promise<Crypto> {
  const ephemeralPrivatePkcs8 = decodeBase64('MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgSgsp3RqsySbAgtzKILKkNNPL+2T5RtBLh7RcGBx99UqhRANCAASrqwXHAT8UqANimfWFVwLEgLaX+cwocIMXcjZPkbMRpblfbgBRoKY5cwnZ7ogMdMyGfLbEGcSxHtR3NpPkvAYa');
  const ephemeralPublicRaw = decodeBase64('BKurBccBPxSoA2KZ9YVXAsSAtpf5zChwgxdyNk+RsxGluV9uAFGgpjlzCdnuiAx0zIZ8tsQZxLEe1Hc2k+S8Bho=');
  const contentKey = await crypto.subtle.importKey('raw', Uint8Array.from({ length: 32 }, (_, index) => index + 1), 'AES-GCM', true, ['encrypt']);
  const ephemeralPrivateKey = await crypto.subtle.importKey('pkcs8', ephemeralPrivatePkcs8, { name: 'ECDH', namedCurve: 'P-256' }, false, ['deriveBits']);
  const ephemeralPublicKey = await crypto.subtle.importKey('raw', ephemeralPublicRaw, { name: 'ECDH', namedCurve: 'P-256' }, true, []);
  const entropy = [
    Uint8Array.from({ length: 18 }, (_, index) => 0x10 + index),
    Uint8Array.from({ length: 12 }, (_, index) => 0x30 + index),
    Uint8Array.from({ length: 32 }, (_, index) => 0x40 + index),
    Uint8Array.from({ length: 12 }, (_, index) => 0x70 + index)
  ];
  let entropyIndex = 0;
  const subtle = new Proxy(crypto.subtle, {
    get(target, property) {
      if (property === 'generateKey') {
        return async (algorithm: AlgorithmIdentifier) =>
          typeof algorithm === 'object' && algorithm.name === 'AES-GCM'
            ? contentKey
            : { privateKey: ephemeralPrivateKey, publicKey: ephemeralPublicKey };
      }
      const value = Reflect.get(target, property);
      return typeof value === 'function' ? value.bind(target) : value;
    }
  });
  return new Proxy(crypto, {
    get(target, property) {
      if (property === 'subtle') return subtle;
      if (property === 'getRandomValues') return <T extends ArrayBufferView>(array: T): T => {
        const value = entropy[entropyIndex++];
        if (!value || value.byteLength !== array.byteLength) throw new Error('unexpected entropy request');
        new Uint8Array(array.buffer, array.byteOffset, array.byteLength).set(value);
        return array;
      };
      const value = Reflect.get(target, property);
      return typeof value === 'function' ? value.bind(target) : value;
    }
  });
}

describe('encrypted job envelopes', () => {
  it('matches the TypeScript-to-Rust full-envelope conformance vector', async () => {
    const fixture = JSON.parse(readFileSync(new URL('../../../contracts/fixtures/encrypted-job-v3.json', import.meta.url), 'utf8'));
    const envelope = await encryptJobContent(Buffer.from(fixture.plaintext, 'base64url'), {
      workspace_id: 'wsp_conformance', environment_id: 'env_conformance', content_type: 'pdf',
      printer_id: 'prt_conformance', target_id: 'tgt_conformance', profile_revision: 'prf_conformance:7',
      options: { copies: 2, duplex: 'long-edge', native_options: { quality: 'high', tray: '2' } },
      deliveries: 2, expires_at: '2099-01-01T00:00:00Z', raw_authorized: false,
      recipients: [{ key_id: 'cek_conformance_1', algorithm: 'ECDH-P256-HKDF-SHA256', public_key_spki: fixture.recipient_public_key_spki }],
      crypto: await deterministicConformanceCrypto()
    });
    expect(envelope).toEqual(fixture.envelope);
  });
  it('uses the exact OpenAPI v3 profile identifiers', () => {
    expect(ENCRYPTED_JOB_V3_VERSION).toBe('piqae-encrypted-job-v3');
    expect(ENCRYPTED_JOB_V3_SUITE).toBe('ECDH-ES-P256+HKDF-SHA256+A256GCMKW+A256GCM');
    expect(ENCRYPTED_JOB_V3_RECIPIENT_ALGORITHM).toBe('ECDH-ES-P256+HKDF-SHA256+A256GCMKW');
  });
  it('isolates concurrent SDK senders targeting the same node key', async () => {
    const keys = await crypto.subtle.generateKey(
      { name: 'ECDH', namedCurve: 'P-256' }, true, ['deriveBits']
    );
    const recipient = {
      key_id: 'cek_shared_node',
      algorithm: 'ECDH-P256-HKDF-SHA256' as const,
      public_key_spki: base64url(await crypto.subtle.exportKey('spki', keys.publicKey))
    };
    const shared = {
      workspace_id: 'wsp_concurrent', environment_id: 'env_concurrent', content_type: 'pdf' as const,
      printer_id: 'prt_concurrent', target_id: 'tgt_concurrent', profile_revision: 'prf_concurrent:1',
      deliveries: 1, expires_at: '2099-01-01T00:00:00Z', raw_authorized: false,
      recipients: [recipient]
    };
    const [senderA, senderB] = await Promise.all([
      encryptJobContent(new TextEncoder().encode('sender A'), shared),
      encryptJobContent(new TextEncoder().encode('sender B'), shared)
    ]);
    expect(senderA.binding.envelope_id).not.toBe(senderB.binding.envelope_id);
    expect(senderA.iv).not.toBe(senderB.iv);
    expect(senderA.ciphertext).not.toBe(senderB.ciphertext);
    expect(senderA.recipients[0]!.ephemeral_public_key).not.toBe(senderB.recipients[0]!.ephemeral_public_key);
    expect(await Promise.all([verifyEncryptedJobEnvelope(senderA), verifyEncryptedJobEnvelope(senderB)])).toEqual([true, true]);
  });
  it('orders native option keys by UTF-8 bytes like Rust BTreeMap', () => {
    expect(Object.keys(canonicalJobOptions({ native_options: { '😀': 'emoji', 'é': 'accent', z: 'latin' } }).native_options)).toEqual(['z', 'é', '😀']);
  });
  it('matches the Rust canonical AAD golden vector', () => {
    const binding = { envelope_id: 'env_012345678901234567890123', workspace_id: 'wsp_test', environment_id: 'env_test', content_type: 'pdf' as const, printer_id: 'prt_test', target_id: 'tgt_test', profile_revision: 'prf_test:3', options: { bin: null, collate: null, color: null, copies: null, dpi: null, duplex: null, fit_to_page: null, media: null, nup: null, pages: null, paper: null, rotate: null, native_options: {} }, deliveries: 1, expires_at: '2099-01-01T00:00:00Z', raw_authorized: false };
    expect(new TextDecoder().decode(encryptedJobAdditionalData(binding))).toBe('{"envelope_id":"env_012345678901234567890123","workspace_id":"wsp_test","environment_id":"env_test","content_type":"pdf","printer_id":"prt_test","target_id":"tgt_test","profile_revision":"prf_test:3","options":{"bin":null,"collate":null,"color":null,"copies":null,"dpi":null,"duplex":null,"fit_to_page":null,"media":null,"nup":null,"pages":null,"paper":null,"rotate":null,"native_options":{}},"deliveries":1,"expires_at":"2099-01-01T00:00:00Z","raw_authorized":false}');
  });
  it('matches the Rust HKDF info golden vector exactly', () => {
    expect(Buffer.from(encryptedJobKeyWrapInfo(
      'env_012345678901234567890123',
      'node-key-7'
    )).toString('hex')).toBe(
      '70697161652d636f6e74656e742d6b65792d777261702d763300656e765f303132333435363738393031323334353637383930313233006e6f64652d6b65792d37'
    );
  });
  it('matches the Rust HKDF-SHA256 derived-key golden vector', async () => {
    const inputKeyMaterial = Uint8Array.from({ length: 32 }, (_, index) => index);
    const salt = Uint8Array.from({ length: 32 }, (_, index) => index + 32);
    const derived = await crypto.subtle.deriveBits(
      {
        name: 'HKDF',
        hash: 'SHA-256',
        salt,
        info: encryptedJobKeyWrapInfo('env_012345678901234567890123', 'node-key-7')
      },
      await crypto.subtle.importKey('raw', inputKeyMaterial, 'HKDF', false, ['deriveBits']),
      256
    );
    expect(Buffer.from(derived).toString('hex')).toBe(
      '92482a2128d133a7345f57033bbeecef47369964ff3d625a3de73fe9aee5e002'
    );
  });
  it('encrypts once, wraps the content key to a dedicated recipient, and binds production metadata', async () => {
    const keys = await crypto.subtle.generateKey(
      { name: 'ECDH', namedCurve: 'P-256' },
      true,
      ['deriveBits']
    );
    const publicKey = await crypto.subtle.exportKey('spki', keys.publicKey);
    const plaintext = new TextEncoder().encode('%PDF-private fixture');
    const envelope = await encryptJobContent(plaintext, {
      content_type: 'pdf',
      workspace_id: 'wsp_test', environment_id: 'env_test', printer_id: 'prt_test', deliveries: 1, raw_authorized: false,
      target_id: 'target_labels',
      profile_revision: 'profile-revision-7',
      expires_at: '2026-09-01T00:00:00Z',
      recipients: [{
        key_id: 'node-encryption-key-3',
        algorithm: 'ECDH-P256-HKDF-SHA256',
        public_key_spki: base64url(publicKey)
      }]
    });

    expect(envelope.version).toBe('piqae-encrypted-job-v3');
    expect(envelope.ciphertext).not.toContain('PDF');
    expect(await verifyEncryptedJobEnvelope(envelope)).toBe(true);

    const wrappedRecipient = envelope.recipients[0]!;
    const ephemeralPublicKey = await crypto.subtle.importKey(
      'raw',
      Buffer.from(wrappedRecipient.ephemeral_public_key, 'base64url'),
      { name: 'ECDH', namedCurve: 'P-256' },
      false,
      []
    );
    const sharedSecret = await crypto.subtle.deriveBits(
      { name: 'ECDH', public: ephemeralPublicKey },
      keys.privateKey,
      256
    );
    const hkdfKey = await crypto.subtle.importKey('raw', sharedSecret, 'HKDF', false, ['deriveKey']);
    const wrapKey = await crypto.subtle.deriveKey(
      {
        name: 'HKDF', hash: 'SHA-256',
        salt: Buffer.from(wrappedRecipient.hkdf_salt, 'base64url'),
        info: encryptedJobKeyWrapInfo(envelope.binding.envelope_id, wrappedRecipient.key_id)
      },
      hkdfKey,
      { name: 'AES-GCM', length: 256 },
      false,
      ['decrypt']
    );
    const contentKey = await crypto.subtle.decrypt(
      {
        name: 'AES-GCM',
        iv: Buffer.from(wrappedRecipient.key_wrap_iv, 'base64url'),
        additionalData: encryptedJobAdditionalData(envelope.binding),
        tagLength: 128
      },
      wrapKey,
      Buffer.from(wrappedRecipient.encrypted_content_key, 'base64url')
    );
    const decrypted = await crypto.subtle.decrypt(
      {
        name: 'AES-GCM',
        iv: new Uint8Array(Buffer.from(envelope.iv, 'base64url')),
        additionalData: encryptedJobAdditionalData(envelope.binding),
        tagLength: 128
      },
      await crypto.subtle.importKey('raw', contentKey, 'AES-GCM', false, ['decrypt']),
      Buffer.from(envelope.ciphertext, 'base64url')
    );
    expect(new Uint8Array(decrypted)).toEqual(plaintext);

    envelope.binding.target_id = 'target-attacker';
    await expect(
      crypto.subtle.decrypt(
        {
          name: 'AES-GCM',
          iv: new Uint8Array(Buffer.from(envelope.iv, 'base64url')),
          additionalData: encryptedJobAdditionalData(envelope.binding),
          tagLength: 128
        },
        await crypto.subtle.importKey('raw', contentKey, 'AES-GCM', false, ['decrypt']),
        Buffer.from(envelope.ciphertext, 'base64url')
      )
    ).rejects.toThrow();
  });

  it('rejects duplicate recipients and detects ciphertext changes', async () => {
    const keys = await crypto.subtle.generateKey(
      { name: 'ECDH', namedCurve: 'P-256' },
      true,
      ['deriveBits']
    );
    const recipient = {
      key_id: 'duplicate',
      algorithm: 'ECDH-P256-HKDF-SHA256' as const,
      public_key_spki: base64url(await crypto.subtle.exportKey('spki', keys.publicKey))
    };
    await expect(encryptJobContent(new Uint8Array([1]), {
      content_type: 'raw', workspace_id: 'wsp', environment_id: 'env', printer_id: 'prt', deliveries: 1, raw_authorized: true, target_id: 'target', profile_revision: 'rev', expires_at: '2026-09-01T00:00:00Z', recipients: [recipient, recipient]
    })).rejects.toThrow(/unique/);

    const envelope = await encryptJobContent(new Uint8Array([1, 2, 3]), {
      content_type: 'raw', workspace_id: 'wsp', environment_id: 'env', printer_id: 'prt', deliveries: 1, raw_authorized: true, target_id: 'target', profile_revision: 'rev', expires_at: '2026-09-01T00:00:00Z', recipients: [recipient]
    });
    const firstCharacter = envelope.ciphertext[0];
    envelope.ciphertext = `${firstCharacter === 'A' ? 'B' : 'A'}${envelope.ciphertext.slice(1)}`;
    expect(await verifyEncryptedJobEnvelope(envelope)).toBe(false);
  });
});
