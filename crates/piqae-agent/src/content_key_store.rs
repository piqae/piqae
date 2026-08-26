//! Durable, bounded storage for document-decryption private-key generations.
//!
//! The manifest contains key identifiers and lifecycle only. macOS and Windows
//! keep every scalar in the current-user credential store; headless Unix keeps
//! each generation in a separate owner-only file.

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use p256::{SecretKey, pkcs8::EncodePublicKey as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt as _;
use std::{
    collections::BTreeMap,
    io::Write as _,
    path::{Path, PathBuf},
};
use zeroize::Zeroize as _;

#[cfg(all(not(test), any(target_os = "macos", target_os = "ios", windows)))]
const SERVICE: &str = "io.piqae.node.content-encryption";
const MANIFEST_VERSION: u8 = 1;
const MAX_GENERATIONS: usize = 8;
const MAX_ENCODED_KEY_BYTES: u64 = 16 * 1024;
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyringManifest {
    version: u8,
    identity_hash: String,
    active_key_id: String,
    decrypt_only_key_ids: Vec<String>,
}

#[derive(Clone)]
pub struct ContentKeyring {
    active_key_id: String,
    keys: BTreeMap<String, SecretKey>,
    decrypt_only_key_ids: Vec<String>,
}

impl std::fmt::Debug for ContentKeyring {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ContentKeyring")
            .field("active_key_id", &self.active_key_id)
            .field("decrypt_only_key_ids", &self.decrypt_only_key_ids)
            .finish_non_exhaustive()
    }
}

impl ContentKeyring {
    pub fn active(&self) -> Option<(&str, &SecretKey)> {
        self.keys
            .get(&self.active_key_id)
            .map(|private| (self.active_key_id.as_str(), private))
    }

    pub fn key(&self, key_id: &str) -> Option<&SecretKey> {
        self.keys.get(key_id)
    }

    pub fn select_key_id<'a>(
        &self,
        recipient_ids: impl IntoIterator<Item = &'a str>,
    ) -> Option<&str> {
        let recipient_ids = recipient_ids
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        if recipient_ids.contains(self.active_key_id.as_str()) {
            return Some(&self.active_key_id);
        }
        self.decrypt_only_key_ids
            .iter()
            .find(|key_id| recipient_ids.contains(key_id.as_str()))
            .map(String::as_str)
    }

    #[cfg(test)]
    fn generation_count(&self) -> usize {
        self.keys.len()
    }
}

pub fn load_or_create(path: &Path, stable_identity: &str) -> Result<ContentKeyring> {
    if stable_identity.trim().is_empty() {
        bail!("content-key identity must not be empty");
    }
    let manifest_path = manifest_path(path);
    #[cfg(windows)]
    recover_manifest_backup(&manifest_path)?;
    if manifest_path.exists() {
        return load_manifest_keyring(path, stable_identity, &manifest_path);
    }

    // This is the only path permitted to create a key. Once a manifest exists,
    // missing generation material always fails closed.
    let private = load_or_create_legacy_private(path)?;
    let key_id = key_id(&private)?;
    persist_generation(path, stable_identity, &key_id, &private)?;
    let manifest = KeyringManifest {
        version: MANIFEST_VERSION,
        identity_hash: identity_hash(stable_identity),
        active_key_id: key_id,
        decrypt_only_key_ids: Vec::new(),
    };
    write_manifest_atomic(&manifest_path, &manifest)?;
    remove_verified_legacy_file(path, &private)?;
    load_manifest_keyring(path, stable_identity, &manifest_path)
}

/// Creates a new active generation while retaining every prior generation for
/// decryption. This is intentionally not wired to an unauthenticated runtime
/// interface.
#[allow(dead_code)]
pub fn rotate(path: &Path, stable_identity: &str) -> Result<ContentKeyring> {
    let current = load_or_create(path, stable_identity)?;
    if current.keys.len() >= MAX_GENERATIONS {
        bail!("content-key generation cap reached; refusing to discard a decryption key");
    }
    let private = generate_private();
    let new_key_id = key_id(&private)?;
    persist_generation(path, stable_identity, &new_key_id, &private)?;
    let mut decrypt_only_key_ids = vec![current.active_key_id];
    decrypt_only_key_ids.extend(current.decrypt_only_key_ids);
    let manifest = KeyringManifest {
        version: MANIFEST_VERSION,
        identity_hash: identity_hash(stable_identity),
        active_key_id: new_key_id,
        decrypt_only_key_ids,
    };
    write_manifest_atomic(&manifest_path(path), &manifest)?;
    load_manifest_keyring(path, stable_identity, &manifest_path(path))
}

fn load_manifest_keyring(
    path: &Path,
    stable_identity: &str,
    manifest_path: &Path,
) -> Result<ContentKeyring> {
    let manifest = read_manifest(manifest_path)?;
    if manifest.version != MANIFEST_VERSION
        || manifest.identity_hash != identity_hash(stable_identity)
        || manifest.active_key_id.is_empty()
        || manifest.decrypt_only_key_ids.len() + 1 > MAX_GENERATIONS
    {
        bail!("content-key manifest is invalid or belongs to another node identity");
    }
    let mut ids = Vec::with_capacity(manifest.decrypt_only_key_ids.len() + 1);
    ids.push(manifest.active_key_id.clone());
    ids.extend(manifest.decrypt_only_key_ids.iter().cloned());
    ids.sort();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        bail!("content-key manifest contains duplicate generations");
    }

    let mut keys = BTreeMap::new();
    for id in ids {
        let private = load_generation(path, stable_identity, &id)
            .with_context(|| format!("load content-key generation {id}"))?;
        if key_id(&private)? != id {
            bail!("content-key generation does not match its manifest identifier");
        }
        keys.insert(id, private);
    }
    Ok(ContentKeyring {
        active_key_id: manifest.active_key_id,
        keys,
        decrypt_only_key_ids: manifest.decrypt_only_key_ids,
    })
}

fn manifest_path(path: &Path) -> PathBuf {
    path.with_extension("keyring.json")
}

fn identity_hash(identity: &str) -> String {
    hex::encode(&Sha256::digest(identity.as_bytes())[..16])
}

fn key_id(private: &SecretKey) -> Result<String> {
    let public_der = private
        .public_key()
        .to_public_key_der()
        .context("encode content encryption public key")?;
    Ok(format!(
        "cek_{}",
        hex::encode(&Sha256::digest(public_der.as_bytes())[..16])
    ))
}

fn read_manifest(path: &Path) -> Result<KeyringManifest> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        bail!("content-key manifest exceeds safe size limit");
    }
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

fn write_manifest_atomic(path: &Path, manifest: &KeyringManifest) -> Result<()> {
    let parent = path
        .parent()
        .context("content-key manifest has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let encoded = serde_json::to_vec(manifest).context("encode content-key manifest")?;
    if encoded.len() as u64 > MAX_MANIFEST_BYTES {
        bail!("content-key manifest exceeds safe size limit");
    }
    let temporary = path.with_extension(format!("tmp-{}", rand::random::<u64>()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&temporary)
        .with_context(|| format!("create {}", temporary.display()))?;
    let result = (|| -> Result<()> {
        file.write_all(&encoded)
            .with_context(|| format!("write {}", temporary.display()))?;
        file.sync_all()
            .with_context(|| format!("sync {}", temporary.display()))?;
        replace_manifest(&temporary, path)?;
        #[cfg(unix)]
        {
            std::fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .with_context(|| format!("sync {}", parent.display()))?;
        }
        let verified = read_manifest(path)?;
        if verified.version != manifest.version
            || verified.identity_hash != manifest.identity_hash
            || verified.active_key_id != manifest.active_key_id
            || verified.decrypt_only_key_ids != manifest.decrypt_only_key_ids
        {
            bail!("persisted content-key manifest failed verification");
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn replace_manifest(temporary: &Path, destination: &Path) -> Result<()> {
    std::fs::rename(temporary, destination)
        .with_context(|| format!("replace {}", destination.display()))
}

#[cfg(windows)]
fn replace_manifest(temporary: &Path, destination: &Path) -> Result<()> {
    let backup = destination.with_extension("keyring.backup");
    if backup.exists() {
        bail!("content-key manifest backup already exists; refusing replacement");
    }
    if destination.exists() {
        std::fs::rename(destination, &backup).context("preserve prior content-key manifest")?;
    }
    if let Err(error) = std::fs::rename(temporary, destination) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, destination);
        }
        return Err(error).context("install replacement content-key manifest");
    }
    if backup.exists() {
        std::fs::remove_file(&backup).context("remove replaced content-key manifest backup")?;
    }
    Ok(())
}

#[cfg(windows)]
fn recover_manifest_backup(manifest: &Path) -> Result<()> {
    recover_manifest_backup_files(manifest)
}

#[cfg(any(windows, test))]
fn recover_manifest_backup_files(manifest: &Path) -> Result<()> {
    let backup = manifest.with_extension("keyring.backup");
    if !backup.exists() {
        return Ok(());
    }
    if manifest.exists() {
        std::fs::remove_file(&backup)
            .context("remove stale content-key manifest backup after replacement")?;
    } else {
        std::fs::rename(&backup, manifest).context("recover prior content-key manifest")?;
    }
    Ok(())
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "ios", windows))))]
fn generation_path(path: &Path, key_id: &str) -> PathBuf {
    path.with_extension(format!("{key_id}.key"))
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "ios", windows))))]
fn persist_generation(path: &Path, _identity: &str, id: &str, private: &SecretKey) -> Result<()> {
    let destination = generation_path(path, id);
    if destination.exists() {
        let existing = load_private_file(&destination)?;
        if existing.to_bytes() != private.to_bytes() {
            bail!("stored content-key generation differs from the requested key");
        }
        return Ok(());
    }
    write_new_secret_file(
        &destination,
        URL_SAFE_NO_PAD.encode(private.to_bytes()).as_bytes(),
    )?;
    let verified = load_private_file(&destination)?;
    if verified.to_bytes() != private.to_bytes() {
        bail!("persisted content-key generation failed verification");
    }
    Ok(())
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "ios", windows))))]
fn load_generation(path: &Path, _identity: &str, id: &str) -> Result<SecretKey> {
    load_private_file(&generation_path(path, id))
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "ios", windows)))]
fn generation_account(identity: &str, key_id: &str) -> String {
    let digest = Sha256::digest(format!("{identity}\0{key_id}").as_bytes());
    format!("node-key-{}", hex::encode(&digest[..16]))
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "ios", windows)))]
fn persist_generation(_path: &Path, identity: &str, id: &str, private: &SecretKey) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, &generation_account(identity, id))
        .context("open OS content-key generation store")?;
    persist_and_verify(&entry, private)
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "ios", windows)))]
fn load_generation(_path: &Path, identity: &str, id: &str) -> Result<SecretKey> {
    let entry = keyring::Entry::new(SERVICE, &generation_account(identity, id))
        .context("open OS content-key generation store")?;
    let mut bytes = entry
        .get_secret()
        .context("read OS-protected content-key generation")?;
    let parsed = parse_private(&bytes);
    bytes.zeroize();
    parsed
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "ios", windows)))]
fn persist_and_verify(entry: &keyring::Entry, private: &SecretKey) -> Result<()> {
    let document = private.to_bytes();
    entry
        .set_secret(document.as_slice())
        .context("store content-key generation")?;
    let mut stored = entry
        .get_secret()
        .context("verify content-key generation")?;
    let matches = stored == document.as_slice();
    stored.zeroize();
    if !matches {
        bail!("OS credential store returned a different content-key generation");
    }
    Ok(())
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "ios", windows)))]
fn load_or_create_legacy_private(path: &Path) -> Result<SecretKey> {
    let entry = keyring::Entry::new(SERVICE, &legacy_credential_account(path))
        .context("open legacy OS content-key store")?;
    match entry.get_secret() {
        Ok(mut bytes) => {
            let result = parse_private(&bytes);
            bytes.zeroize();
            result
        }
        Err(keyring::Error::NoEntry) if path.exists() => {
            let private = load_private_file(path)?;
            persist_and_verify(&entry, &private)?;
            Ok(private)
        }
        Err(keyring::Error::NoEntry) if marker_path(path).exists() => {
            bail!("OS-protected content encryption key is missing; refusing to replace it")
        }
        Err(keyring::Error::NoEntry) => {
            let private = generate_private();
            persist_and_verify(&entry, &private)?;
            write_marker(path)?;
            Ok(private)
        }
        Err(error) => Err(error).context("read legacy OS content-key store"),
    }
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "ios", windows))))]
fn load_or_create_legacy_private(path: &Path) -> Result<SecretKey> {
    if path.exists() {
        return load_private_file(path);
    }
    let private = generate_private();
    write_new_secret_file(path, URL_SAFE_NO_PAD.encode(private.to_bytes()).as_bytes())?;
    Ok(private)
}

fn load_private_file(path: &Path) -> Result<SecretKey> {
    let mut encoded = read_bounded_key_file(path)?;
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded.trim())
        .context("decode content encryption key");
    encoded.zeroize();
    let mut bytes = decoded?;
    let parsed = parse_private(&bytes);
    bytes.zeroize();
    parsed
}

fn remove_verified_legacy_file(path: &Path, private: &SecretKey) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let existing = load_private_file(path)?;
    if existing.to_bytes() != private.to_bytes() {
        bail!("legacy content key differs from persisted generation; refusing removal");
    }
    std::fs::remove_file(path)
        .with_context(|| format!("remove verified legacy key {}", path.display()))
}

fn read_bounded_key_file(path: &Path) -> Result<String> {
    let metadata =
        std::fs::metadata(path).with_context(|| format!("inspect {}", path.display()))?;
    if metadata.len() > MAX_ENCODED_KEY_BYTES {
        bail!("content encryption key file exceeds safe size limit");
    }
    std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
}

#[cfg(any(test, not(any(target_os = "macos", target_os = "ios", windows))))]
fn write_new_secret_file(path: &Path, secret: &[u8]) -> Result<()> {
    let parent = path.parent().context("content-key file has no parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(secret)
        .with_context(|| format!("write {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync {}", path.display()))
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

#[cfg(all(not(test), any(target_os = "macos", target_os = "ios", windows)))]
fn marker_path(path: &Path) -> PathBuf {
    path.with_extension("content-encryption.os-protected")
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "ios", windows)))]
fn write_marker(path: &Path) -> Result<()> {
    let marker = marker_path(path);
    if marker.exists() {
        return Ok(());
    }
    let parent = marker
        .parent()
        .context("content-key marker has no parent")?;
    std::fs::create_dir_all(parent)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&marker)?;
    file.write_all(b"piqae-os-content-key-v1\n")?;
    file.sync_all()?;
    Ok(())
}

#[cfg(all(not(test), any(target_os = "macos", target_os = "ios", windows)))]
fn legacy_credential_account(path: &Path) -> String {
    let digest = Sha256::digest(path.as_os_str().to_string_lossy().as_bytes());
    format!("installation-{}", hex::encode(&digest[..16]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn existing_single_key_migrates_without_rotation_and_survives_restart() -> Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("content-encryption.key");
        let private = generate_private();
        write_new_secret_file(&path, URL_SAFE_NO_PAD.encode(private.to_bytes()).as_bytes())?;
        let expected = key_id(&private)?;
        let first = load_or_create(&path, "agt_stable")?;
        assert_eq!(
            first.active().map(|active| active.0),
            Some(expected.as_str())
        );
        let second = load_or_create(&path, "agt_stable")?;
        assert_eq!(
            second.active().map(|active| active.0),
            Some(expected.as_str())
        );
        assert_eq!(second.generation_count(), 1);
        Ok(())
    }

    #[test]
    fn rotation_retains_old_recipient_after_restart() -> Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("content-encryption.key");
        let initial = load_or_create(&path, "agt_stable")?;
        let old_id = initial.active().context("active key")?.0.to_owned();
        let rotated = rotate(&path, "agt_stable")?;
        assert_ne!(rotated.active().context("rotated active key")?.0, old_id);
        assert!(rotated.key(&old_id).is_some());
        let restarted = load_or_create(&path, "agt_stable")?;
        assert!(restarted.key(&old_id).is_some());
        assert_eq!(restarted.generation_count(), 2);
        assert_eq!(
            restarted.select_key_id([
                old_id.as_str(),
                restarted.active().context("restarted active key")?.0,
            ]),
            Some(restarted.active().context("restarted active key")?.0),
        );
        assert_eq!(
            restarted.select_key_id([old_id.as_str()]),
            Some(old_id.as_str())
        );
        Ok(())
    }

    #[test]
    fn missing_or_corrupt_prior_generation_never_regenerates() -> Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("content-encryption.key");
        let keyring = load_or_create(&path, "agt_stable")?;
        let active_id = keyring.active().context("active key")?.0.to_owned();
        #[cfg(any(test, not(any(target_os = "macos", target_os = "ios", windows))))]
        std::fs::remove_file(generation_path(&path, &active_id))?;
        assert!(load_or_create(&path, "agt_stable").is_err());
        assert!(manifest_path(&path).exists());
        Ok(())
    }

    #[test]
    fn corrupt_manifest_and_changed_identity_fail_closed() -> Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("content-encryption.key");
        load_or_create(&path, "agt_stable")?;
        assert!(load_or_create(&path, "agt_other").is_err());
        std::fs::write(manifest_path(&path), b"{")?;
        assert!(load_or_create(&path, "agt_stable").is_err());
        Ok(())
    }

    #[test]
    fn generation_cap_refuses_rotation_without_discarding_keys() -> Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("content-encryption.key");
        let mut keyring = load_or_create(&path, "agt_stable")?;
        while keyring.generation_count() < MAX_GENERATIONS {
            keyring = rotate(&path, "agt_stable")?;
        }
        assert!(rotate(&path, "agt_stable").is_err());
        assert_eq!(
            load_or_create(&path, "agt_stable")?.generation_count(),
            MAX_GENERATIONS
        );
        Ok(())
    }

    #[test]
    fn manifest_backup_recovery_restores_or_removes_as_required() -> Result<()> {
        let directory = tempdir()?;
        let manifest = directory.path().join("content-encryption.keyring.json");
        let backup = manifest.with_extension("keyring.backup");

        std::fs::write(&backup, b"prior")?;
        recover_manifest_backup_files(&manifest)?;
        assert_eq!(std::fs::read(&manifest)?, b"prior");
        assert!(!backup.exists());

        std::fs::write(&manifest, b"current")?;
        std::fs::write(&backup, b"stale")?;
        recover_manifest_backup_files(&manifest)?;
        assert_eq!(std::fs::read(&manifest)?, b"current");
        assert!(!backup.exists());
        Ok(())
    }

    #[test]
    fn generated_key_round_trips_scalar_encoding() -> Result<()> {
        let private = generate_private();
        let parsed = parse_private(private.to_bytes().as_slice())?;
        assert_eq!(private.public_key(), parsed.public_key());
        Ok(())
    }
}
