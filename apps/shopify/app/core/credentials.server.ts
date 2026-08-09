import { createCipheriv, createDecipheriv, randomBytes } from "node:crypto";
import { normalizeShopDomain } from "./model";

export class CredentialVault {
  constructor(private readonly key: Buffer) {
    if (key.length !== 32)
      throw new Error("credential encryption key must be 32 bytes");
  }
  static fromBase64(value: string) {
    return new CredentialVault(Buffer.from(value, "base64"));
  }
  seal(value: string, shop: string): string {
    const nonce = randomBytes(12);
    const cipher = createCipheriv("aes-256-gcm", this.key, nonce);
    cipher.setAAD(
      Buffer.from(`piqae-shopify-credential/v1\0${normalizeShopDomain(shop)}`),
    );
    const ciphertext = Buffer.concat([
      cipher.update(value, "utf8"),
      cipher.final(),
    ]);
    return [
      "v1",
      nonce.toString("base64url"),
      cipher.getAuthTag().toString("base64url"),
      ciphertext.toString("base64url"),
    ].join(".");
  }
  open(envelope: string, shop: string): string {
    const [version, nonce, tag, ciphertext, extra] = envelope.split(".");
    if (version !== "v1" || !nonce || !tag || !ciphertext || extra)
      throw new Error("invalid credential envelope");
    const decipher = createDecipheriv(
      "aes-256-gcm",
      this.key,
      Buffer.from(nonce, "base64url"),
    );
    decipher.setAAD(
      Buffer.from(`piqae-shopify-credential/v1\0${normalizeShopDomain(shop)}`),
    );
    decipher.setAuthTag(Buffer.from(tag, "base64url"));
    return Buffer.concat([
      decipher.update(Buffer.from(ciphertext, "base64url")),
      decipher.final(),
    ]).toString("utf8");
  }
}
