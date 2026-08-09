import { createCipheriv, createDecipheriv, randomBytes } from "node:crypto";
export interface DownloadGrant {
  shop: string;
  renderId: string;
  orderGid: string;
  customerGid: string;
  expiresAt: number;
}
export interface PreviewGrant {
  shop: string;
  renderId: string;
  previewId: string;
  expiresAt: number;
}
export class DownloadTokenVault {
  constructor(private readonly key: Buffer) {
    if (key.length !== 32) throw new Error("download key must be 32 bytes");
  }
  issue(grant: Omit<DownloadGrant, "expiresAt">, ttlSeconds = 300): string {
    const nonce = randomBytes(12);
    const cipher = createCipheriv("aes-256-gcm", this.key, nonce);
    cipher.setAAD(Buffer.from("piqae-shopify-download/v1"));
    const encrypted = Buffer.concat([
      cipher.update(
        Buffer.from(
          JSON.stringify({
            ...grant,
            expiresAt: Date.now() + ttlSeconds * 1000,
          }),
        ),
      ),
      cipher.final(),
    ]);
    return [
      "v1",
      nonce.toString("base64url"),
      cipher.getAuthTag().toString("base64url"),
      encrypted.toString("base64url"),
    ].join(".");
  }
  open(token: string): DownloadGrant {
    const [version, nonce, tag, ciphertext, extra] = token.split(".");
    if (version !== "v1" || !nonce || !tag || !ciphertext || extra)
      throw new Error("invalid download token");
    const decipher = createDecipheriv(
      "aes-256-gcm",
      this.key,
      Buffer.from(nonce, "base64url"),
    );
    decipher.setAAD(Buffer.from("piqae-shopify-download/v1"));
    decipher.setAuthTag(Buffer.from(tag, "base64url"));
    const grant = JSON.parse(
      Buffer.concat([
        decipher.update(Buffer.from(ciphertext, "base64url")),
        decipher.final(),
      ]).toString("utf8"),
    ) as DownloadGrant;
    if (!Number.isSafeInteger(grant.expiresAt) || grant.expiresAt < Date.now())
      throw new Error("download token expired");
    return grant;
  }

  issuePreview(
    grant: Omit<PreviewGrant, "expiresAt">,
    ttlSeconds = 900,
  ): string {
    return this.seal("piqae-shopify-preview/v1", {
      ...grant,
      expiresAt: Date.now() + ttlSeconds * 1000,
    });
  }

  openPreview(token: string): PreviewGrant {
    const grant = this.unseal(
      "piqae-shopify-preview/v1",
      token,
    ) as PreviewGrant;
    if (!Number.isSafeInteger(grant.expiresAt) || grant.expiresAt < Date.now())
      throw new Error("preview token expired");
    return grant;
  }

  private seal(aad: string, value: object): string {
    const nonce = randomBytes(12);
    const cipher = createCipheriv("aes-256-gcm", this.key, nonce);
    cipher.setAAD(Buffer.from(aad));
    const encrypted = Buffer.concat([
      cipher.update(Buffer.from(JSON.stringify(value))),
      cipher.final(),
    ]);
    return [
      "v1",
      nonce.toString("base64url"),
      cipher.getAuthTag().toString("base64url"),
      encrypted.toString("base64url"),
    ].join(".");
  }

  private unseal(aad: string, token: string): object {
    const [version, nonce, tag, ciphertext, extra] = token.split(".");
    if (version !== "v1" || !nonce || !tag || !ciphertext || extra)
      throw new Error("invalid preview token");
    const decipher = createDecipheriv(
      "aes-256-gcm",
      this.key,
      Buffer.from(nonce, "base64url"),
    );
    decipher.setAAD(Buffer.from(aad));
    decipher.setAuthTag(Buffer.from(tag, "base64url"));
    return JSON.parse(
      Buffer.concat([
        decipher.update(Buffer.from(ciphertext, "base64url")),
        decipher.final(),
      ]).toString("utf8"),
    ) as object;
  }
}
