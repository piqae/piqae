use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Payload},
};
use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hkdf::Hkdf;
use p256::{SecretKey, ecdh::diffie_hellman, pkcs8::DecodePrivateKey};
use piqae_domain::{EncryptedContentManifest, EncryptedContentRecipient};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Deserialize)]
struct FixtureEnvelope {
    #[serde(flatten)]
    manifest: EncryptedContentManifest,
    ciphertext: String,
}

#[derive(Deserialize)]
struct Fixture {
    recipient_private_key_pkcs8: String,
    plaintext: String,
    envelope: FixtureEnvelope,
}

fn decrypt(private_key: &SecretKey, envelope: &FixtureEnvelope) -> Result<Vec<u8>> {
    let recipient: &EncryptedContentRecipient = envelope
        .manifest
        .recipients
        .first()
        .ok_or_else(|| anyhow!("missing recipient"))?;
    if envelope.manifest.version != piqae_domain::ENCRYPTED_JOB_V3_VERSION
        || envelope.manifest.suite != piqae_domain::ENCRYPTED_JOB_V3_SUITE
        || recipient.algorithm != piqae_domain::ENCRYPTED_JOB_V3_RECIPIENT_ALGORITHM
    {
        return Err(anyhow!("unsupported profile"));
    }
    let ciphertext = URL_SAFE_NO_PAD.decode(&envelope.ciphertext)?;
    if Sha256::digest(&ciphertext).as_slice()
        != URL_SAFE_NO_PAD.decode(&envelope.manifest.ciphertext_sha256)?
    {
        return Err(anyhow!("ciphertext digest mismatch"));
    }
    let ephemeral = p256::PublicKey::from_sec1_bytes(
        &URL_SAFE_NO_PAD.decode(&recipient.ephemeral_public_key)?,
    )?;
    let shared = diffie_hellman(private_key.to_nonzero_scalar(), ephemeral.as_affine());
    let salt = URL_SAFE_NO_PAD.decode(&recipient.hkdf_salt)?;
    let info = format!(
        "piqae-content-key-wrap-v3\0{}\0{}",
        envelope.manifest.binding.envelope_id, recipient.key_id
    );
    let hkdf = Hkdf::<Sha256>::new(Some(&salt), shared.raw_secret_bytes().as_slice());
    let mut wrapping_key = [0_u8; 32];
    hkdf.expand(info.as_bytes(), &mut wrapping_key)
        .map_err(|_| anyhow!("invalid HKDF output length"))?;
    let aad = serde_json::to_vec(&envelope.manifest.binding)?;
    let content_key = Aes256Gcm::new_from_slice(&wrapping_key)?
        .decrypt(
            URL_SAFE_NO_PAD
                .decode(&recipient.key_wrap_iv)?
                .as_slice()
                .into(),
            Payload {
                msg: &URL_SAFE_NO_PAD.decode(&recipient.encrypted_content_key)?,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("content key authentication failed"))?;
    Aes256Gcm::new_from_slice(&content_key)?
        .decrypt(
            URL_SAFE_NO_PAD
                .decode(&envelope.manifest.iv)?
                .as_slice()
                .into(),
            Payload {
                msg: &ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("content authentication failed"))
}

#[test]
fn typescript_envelope_decrypts_in_rust_and_rejects_tampering() -> Result<()> {
    let fixture: Fixture = serde_json::from_str(include_str!(
        "../../../contracts/fixtures/encrypted-job-v3.json"
    ))?;
    let private_key_der = URL_SAFE_NO_PAD.decode(&fixture.recipient_private_key_pkcs8)?;
    let private_key = SecretKey::from_pkcs8_der(&private_key_der)?;
    assert_eq!(
        decrypt(&private_key, &fixture.envelope)?,
        URL_SAFE_NO_PAD.decode(&fixture.plaintext)?
    );

    let mut tampered_binding = fixture.envelope.clone();
    tampered_binding.manifest.binding.deliveries += 1;
    assert!(decrypt(&private_key, &tampered_binding).is_err());

    let mut tampered_recipient = fixture.envelope;
    tampered_recipient.manifest.recipients[0].key_id = "cek_conformance_2".into();
    assert!(decrypt(&private_key, &tampered_recipient).is_err());
    Ok(())
}
