//! Durable storage for document-decryption private keys.
//!
//! macOS and Windows use their current-user OS credential stores. Linux and
//! other Unix service deployments retain an owner-only file because a desktop
//! secret service is not reliably available to a headless daemon.

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::{SecretKey, pkcs8::EncodePublicKey as _};
use sha2::{Digest as _, Sha256};
use std::path::Path;
use zeroize::Zeroize as _;

#[cfg(any(target_os = "macos", target_os = "ios", windows))]
const SERVICE: &str = "io.piqae.node.content-encryption";
const MAX_ENCODED_KEY_BYTES: u64 = 16 * 1024;

pub fn load_or_create(path: &Path) -> Result<(SecretKey, String)> {
    let private = load_or_create_private(path)?;
    let public_der = private
        .public_key()
        .to_public_key_der()
        .context("encode content encryption public key")?;
    let key_id = format!(
        "cek_{}",
        hex::encode(&Sha256::digest(public_der.as_bytes())[..16])
    );
    Ok((private, key_id))
}

#[cfg(any(target_os = "macos", target_os = "ios", windows))]
fn load_or_create_private(path: &Path) -> Result<SecretKey> {
    let account = credential_account(path);
    let entry = keyring::Entry::new(SERVICE, &account).context("open OS content-key store")?;
    match entry.get_secret() {
        Ok(mut der) => {
            let parsed = parse_private(&der).context("parse OS-protected content encryption key");
            let result = parsed.and_then(|private| {
                reconcile_legacy_file(path, &der)?;
                write_marker(path)?;
                Ok(private)
            });
            der.zeroize();
            result
        }
        Err(keyring::Error::NoEntry) if path.exists() => migrate_file(path, &entry),
        Err(keyring::Error::NoEntry) if marker_path(path).exists() => {
            bail!("OS-protected content encryption key is missing; refusing to replace it")
        }
        Err(keyring::Error::NoEntry) => {
            let private = generate_private();
            persist_and_verify(&entry, &private)?;
            write_marker(path)?;
            Ok(private)
        }
        Err(error) => Err(error).context("read OS-protected content encryption key"),
    }
}

#[cfg(any(target_os = "macos", target_os = "ios", windows))]
fn migrate_file(path: &Path, entry: &keyring::Entry) -> Result<SecretKey> {
    let mut encoded = read_bounded_key_file(path)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .context("decode legacy content encryption key");
    encoded.zeroize();
    let mut der = decoded?;
    let parsed = parse_private(&der).context("parse legacy content encryption key");
    der.zeroize();
    let private = parsed?;
    persist_and_verify(entry, &private)?;
    std::fs::remove_file(path).with_context(|| {
        format!(
            "remove migrated plaintext content key {}; OS copy is intact",
            path.display()
        )
    })?;
    write_marker(path)?;
    Ok(private)
}

#[cfg(any(target_os = "macos", target_os = "ios", windows))]
fn reconcile_legacy_file(path: &Path, stored_der: &[u8]) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut encoded = read_bounded_key_file(path)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .context("decode legacy content encryption key");
    encoded.zeroize();
    let mut legacy_der = decoded?;
    let matches = legacy_der == stored_der;
    legacy_der.zeroize();
    if !matches {
        bail!("legacy content key differs from OS-protected key; refusing destructive migration");
    }
    std::fs::remove_file(path)
        .with_context(|| format!("remove verified legacy content key {}", path.display()))
}

#[cfg(any(target_os = "macos", target_os = "ios", windows))]
fn marker_path(path: &Path) -> std::path::PathBuf {
    path.with_extension("content-encryption.os-protected")
}

#[cfg(any(target_os = "macos", target_os = "ios", windows))]
fn write_marker(path: &Path) -> Result<()> {
    use std::io::Write as _;

    let marker = marker_path(path);
    if marker.exists() {
        return Ok(());
    }
    let parent = marker
        .parent()
        .context("content-key marker has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&marker)
        .with_context(|| format!("create {}", marker.display()))?;
    file.write_all(b"piqae-os-content-key-v1\n")
        .with_context(|| format!("write {}", marker.display()))?;
    file.sync_all()
        .with_context(|| format!("sync {}", marker.display()))
}

#[cfg(any(target_os = "macos", target_os = "ios", windows))]
fn persist_and_verify(entry: &keyring::Entry, private: &SecretKey) -> Result<()> {
    let document = private.to_bytes();
    entry
        .set_secret(document.as_slice())
        .context("store content encryption key in OS credential store")?;
    let mut stored = entry
        .get_secret()
        .context("verify OS-protected content encryption key")?;
    let matches = stored == document.as_slice();
    stored.zeroize();
    if !matches {
        bail!("OS credential store returned a different content encryption key");
    }
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "ios", windows)))]
fn load_or_create_private(path: &Path) -> Result<SecretKey> {
    if path.exists() {
        let mut encoded = read_bounded_key_file(path)?;
        let decoded = URL_SAFE_NO_PAD
            .decode(encoded.trim())
            .context("decode content encryption key");
        encoded.zeroize();
        let mut der = decoded?;
        let parsed = parse_private(&der).context("parse content encryption key");
        der.zeroize();
        return parsed;
    }

    let private = generate_private();
    let document = private.to_bytes();
    super::write_new_secret_file(path, URL_SAFE_NO_PAD.encode(document.as_slice()).as_bytes())?;
    Ok(private)
}

fn read_bounded_key_file(path: &Path) -> Result<String> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    if metadata.len() > MAX_ENCODED_KEY_BYTES {
        bail!("content encryption key file exceeds safe size limit");
    }
    std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
}

fn generate_private() -> SecretKey {
    SecretKey::random(&mut rand::rngs::OsRng)
}

fn parse_private(bytes: &[u8]) -> Result<SecretKey> {
    if bytes.len() != 32 {
        bail!(
            "legacy RSA content key cannot be migrated safely; drain encrypted jobs and re-enrol this connector"
        );
    }
    SecretKey::from_slice(bytes).context("invalid P-256 content encryption key")
}

#[cfg(any(target_os = "macos", target_os = "ios", windows, test))]
fn credential_account(path: &Path) -> String {
    let digest = Sha256::digest(path.as_os_str().to_string_lossy().as_bytes());
    format!("installation-{}", hex::encode(&digest[..16]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_account_is_stable_and_does_not_expose_the_path() {
        let path = Path::new("/private/customer/acme/content-encryption.key");
        let account = credential_account(path);
        assert_eq!(account, credential_account(path));
        assert!(!account.contains("customer"));
        assert_ne!(
            account,
            credential_account(Path::new("/private/customer/other/content-encryption.key"))
        );
    }

    #[test]
    fn generated_key_round_trips_scalar_encoding() -> Result<()> {
        let private = generate_private();
        let document = private.to_bytes();
        let parsed = parse_private(document.as_slice())?;
        assert_eq!(private.public_key(), parsed.public_key());
        Ok(())
    }

    #[test]
    fn legacy_rsa_sized_material_fails_closed() {
        let result = parse_private(&vec![0_u8; 1_700]);
        assert!(result.is_err());
        assert!(
            result
                .err()
                .is_some_and(|error| error.to_string().contains("legacy RSA"))
        );
    }
}
