//! Persistent, content-addressed resources for deterministic node rendering.
//!
//! The cache never fetches a URL and never trusts metadata as proof of file
//! integrity. Its caller supplies bytes, which are bounded, hashed, fsynced and
//! atomically published before `SQLite` records them. Concurrent misses for one
//! digest are collapsed into one acquisition.

use piqae_agent_storage::{AgentStore, StorageError, StoredDocumentResource};
use piqae_document_renderer::{BusinessDocumentV1, ResolvedResources, Resource};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs::{self, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use thiserror::Error;
use uuid::Uuid;

pub const RESOURCE_ABI: &str = "piqae.document-resources/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeResourceDescriptor {
    pub digest: String,
    pub media_type: String,
    pub byte_length: u64,
}

#[derive(Debug, Error)]
pub enum ResourceCacheError {
    #[error("invalid document resource descriptor")]
    InvalidDescriptor,
    #[error("document resource exceeds the configured cache bound")]
    Limit,
    #[error("document resource bytes do not match their descriptor")]
    DigestMismatch,
    #[error("document resource acquisition failed: {0}")]
    Acquisition(String),
    #[error("document resource I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("document resource metadata failed: {0}")]
    Storage(#[from] StorageError),
    #[error("document resource cache synchronization failed")]
    Synchronization,
}

#[derive(Debug)]
struct Flight {
    result: Mutex<Option<Result<PathBuf, String>>>,
    ready: Condvar,
}

/// Restart-safe resource cache. `maximum_bytes` includes referenced entries;
/// if pinned resources fill it, new acquisitions fail rather than evicting
/// content still required by a queued job.
#[derive(Debug)]
pub struct DocumentResourceCache {
    root: PathBuf,
    maximum_bytes: u64,
    maximum_resource_bytes: u64,
    store: Mutex<AgentStore>,
    flights: Mutex<HashMap<String, Arc<Flight>>>,
}

impl DocumentResourceCache {
    /// Opens a persistent cache and reconciles metadata with stored bytes.
    ///
    /// # Errors
    /// Returns an error for invalid limits, I/O, metadata, or corrupt state.
    pub fn open(
        root: impl AsRef<Path>,
        database_path: impl AsRef<Path>,
        maximum_bytes: u64,
        maximum_resource_bytes: u64,
    ) -> Result<Self, ResourceCacheError> {
        if maximum_bytes == 0 || maximum_resource_bytes == 0 {
            return Err(ResourceCacheError::Limit);
        }
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("sha256"))?;
        let store = AgentStore::open(database_path)?;
        store.reset_document_resource_transient_state()?;
        let cache = Self {
            root,
            maximum_bytes,
            maximum_resource_bytes,
            store: Mutex::new(store),
            flights: Mutex::new(HashMap::new()),
        };
        cache.reconcile()?;
        Ok(cache)
    }

    /// Returns a verified path, invoking `acquire` at most once for concurrent
    /// callers of the same missing digest.
    ///
    /// # Errors
    /// Returns an error for invalid metadata, acquisition, I/O, or cache limits.
    pub fn resolve<F, R>(
        &self,
        descriptor: &NodeResourceDescriptor,
        acquire: F,
        now_unix_ms: i64,
    ) -> Result<PathBuf, ResourceCacheError>
    where
        F: FnOnce() -> Result<R, String>,
        R: std::io::Read,
    {
        validate_descriptor(descriptor, self.maximum_resource_bytes)?;
        if let Some(path) = self.cached_verified(descriptor, now_unix_ms)? {
            return Ok(path);
        }

        let (flight, leader) = {
            let mut flights = self
                .flights
                .lock()
                .map_err(|_| ResourceCacheError::Synchronization)?;
            let selected = match flights.entry(descriptor.digest.clone()) {
                Entry::Occupied(entry) => (Arc::clone(entry.get()), false),
                Entry::Vacant(entry) => {
                    let flight = Arc::new(Flight {
                        result: Mutex::new(None),
                        ready: Condvar::new(),
                    });
                    entry.insert(Arc::clone(&flight));
                    (flight, true)
                }
            };
            drop(flights);
            selected
        };
        if leader {
            let result = self
                .store_acquired(descriptor, acquire, now_unix_ms)
                .map_err(|error| error.to_string());
            let outward = result.clone().map_err(ResourceCacheError::Acquisition);
            *flight
                .result
                .lock()
                .map_err(|_| ResourceCacheError::Synchronization)? = Some(result);
            flight.ready.notify_all();
            self.flights
                .lock()
                .map_err(|_| ResourceCacheError::Synchronization)?
                .remove(&descriptor.digest);
            outward
        } else {
            let mut result = flight
                .result
                .lock()
                .map_err(|_| ResourceCacheError::Synchronization)?;
            while result.is_none() {
                result = flight
                    .ready
                    .wait(result)
                    .map_err(|_| ResourceCacheError::Synchronization)?;
            }
            match result.as_ref() {
                Some(Ok(path)) => Ok(path.clone()),
                Some(Err(error)) => Err(ResourceCacheError::Acquisition(error.clone())),
                None => Err(ResourceCacheError::Synchronization),
            }
        }
    }

    /// Returns a verified warm-cache path without acquiring missing bytes.
    ///
    /// # Errors
    /// Returns an error for invalid metadata, I/O, or synchronization failure.
    pub fn resolve_existing(
        &self,
        descriptor: &NodeResourceDescriptor,
        now_unix_ms: i64,
    ) -> Result<Option<PathBuf>, ResourceCacheError> {
        validate_descriptor(descriptor, self.maximum_resource_bytes)?;
        self.cached_verified(descriptor, now_unix_ms)
    }

    /// Pins a cached resource against LRU eviction.
    ///
    /// # Errors
    /// Returns an error for an unknown digest or metadata failure.
    pub fn pin(&self, digest: &str) -> Result<(), ResourceCacheError> {
        self.store
            .lock()
            .map_err(|_| ResourceCacheError::Synchronization)?
            .retain_document_resource(digest)?;
        Ok(())
    }

    /// Releases one cached-resource eviction pin.
    ///
    /// # Errors
    /// Returns an error for an unknown/unpinned digest or metadata failure.
    pub fn unpin(&self, digest: &str) -> Result<(), ResourceCacheError> {
        self.store
            .lock()
            .map_err(|_| ResourceCacheError::Synchronization)?
            .release_document_resource(digest)?;
        Ok(())
    }

    /// Returns a bounded inventory of entries whose bytes still verify.
    ///
    /// # Errors
    /// Returns an error for cache I/O, synchronization, or metadata failures.
    pub fn verified_digests(&self, limit: usize) -> Result<Vec<String>, ResourceCacheError> {
        let limit = limit.min(256);
        let store = self
            .store
            .lock()
            .map_err(|_| ResourceCacheError::Synchronization)?;
        let entries = store.recent_document_resources(limit)?;
        drop(store);
        let mut verified = Vec::with_capacity(entries.len());
        for entry in entries {
            let descriptor = NodeResourceDescriptor {
                digest: entry.digest.clone(),
                media_type: entry.media_type,
                byte_length: entry.byte_length,
            };
            if verify_file(&self.root.join(entry.relative_path), &descriptor)? {
                verified.push(entry.digest);
            }
        }
        Ok(verified)
    }

    /// Resolves all currently supported resources in a document and returns
    /// the renderer's verified in-memory resource set. A missing or unsupported
    /// resource fails the whole operation so the caller can use server PDF.
    ///
    /// # Errors
    /// Returns an error for unsupported, missing, corrupt, or oversized assets.
    pub fn resolve_document_images<F>(
        &self,
        document: &BusinessDocumentV1,
        mut acquire: F,
        now_unix_ms: i64,
    ) -> Result<ResolvedResources, ResourceCacheError>
    where
        F: FnMut(&NodeResourceDescriptor) -> Result<Vec<u8>, String>,
    {
        let mut resolved = ResolvedResources::default();
        for (resource_id, resource) in &document.resources {
            let Resource::Image {
                digest,
                media_type,
                byte_length,
            } = resource;
            if media_type != "image/jpeg" {
                return Err(ResourceCacheError::InvalidDescriptor);
            }
            let descriptor = NodeResourceDescriptor {
                digest: digest
                    .strip_prefix("sha256:")
                    .ok_or(ResourceCacheError::InvalidDescriptor)?
                    .to_ascii_lowercase(),
                media_type: media_type.clone(),
                byte_length: *byte_length,
            };
            let path = self.resolve(
                &descriptor,
                || acquire(&descriptor).map(std::io::Cursor::new),
                now_unix_ms,
            )?;
            let bytes = fs::read(path)?;
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != descriptor.byte_length
                || format!("{:x}", Sha256::digest(&bytes)) != descriptor.digest
            {
                return Err(ResourceCacheError::DigestMismatch);
            }
            resolved.images.insert(resource_id.clone(), bytes);
        }
        Ok(resolved)
    }

    fn cached_verified(
        &self,
        descriptor: &NodeResourceDescriptor,
        now: i64,
    ) -> Result<Option<PathBuf>, ResourceCacheError> {
        let metadata = self
            .store
            .lock()
            .map_err(|_| ResourceCacheError::Synchronization)?
            .document_resource(&descriptor.digest)?;
        let Some(metadata) = metadata else {
            return Ok(None);
        };
        let path = self.root.join(&metadata.relative_path);
        if metadata.byte_length == descriptor.byte_length
            && metadata.media_type == descriptor.media_type
            && verify_file(&path, descriptor)?
        {
            self.store
                .lock()
                .map_err(|_| ResourceCacheError::Synchronization)?
                .touch_document_resource(&descriptor.digest, now)?;
            return Ok(Some(path));
        }
        if metadata.reference_count > 0 {
            return Err(ResourceCacheError::DigestMismatch);
        }
        let _ = fs::remove_file(&path);
        self.store
            .lock()
            .map_err(|_| ResourceCacheError::Synchronization)?
            .delete_unreferenced_document_resource(&descriptor.digest)?;
        Ok(None)
    }

    fn store_acquired<F, R>(
        &self,
        descriptor: &NodeResourceDescriptor,
        acquire: F,
        now: i64,
    ) -> Result<PathBuf, ResourceCacheError>
    where
        F: FnOnce() -> Result<R, String>,
        R: std::io::Read,
    {
        let reader = acquire().map_err(ResourceCacheError::Acquisition)?;
        let read_limit = descriptor
            .byte_length
            .min(self.maximum_resource_bytes)
            .saturating_add(1);
        let mut bytes = Vec::with_capacity(
            usize::try_from(descriptor.byte_length.min(64 * 1024)).unwrap_or(64 * 1024),
        );
        reader.take(read_limit).read_to_end(&mut bytes)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != descriptor.byte_length
            || format!("{:x}", Sha256::digest(&bytes)) != descriptor.digest.to_ascii_lowercase()
        {
            return Err(ResourceCacheError::DigestMismatch);
        }
        self.evict_for(descriptor.byte_length)?;
        let relative = relative_path(&descriptor.digest);
        let final_path = self.root.join(&relative);
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = final_path.with_extension(format!("tmp-{}", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        if let Err(error) = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &final_path)?;
            if let Some(parent) = final_path.parent() {
                OpenOptions::new().read(true).open(parent)?.sync_all()?;
            }
            Ok::<(), std::io::Error>(())
        })() {
            let _ = fs::remove_file(&temporary);
            return Err(error.into());
        }
        let metadata = StoredDocumentResource {
            digest: descriptor.digest.to_ascii_lowercase(),
            media_type: descriptor.media_type.clone(),
            byte_length: descriptor.byte_length,
            relative_path: relative.to_string_lossy().into_owned(),
            verified_unix_ms: now,
            last_accessed_unix_ms: now,
            reference_count: 0,
        };
        let persisted = self
            .store
            .lock()
            .map_err(|_| ResourceCacheError::Synchronization)?
            .upsert_document_resource(&metadata);
        if let Err(error) = persisted {
            let _ = fs::remove_file(&final_path);
            return Err(error.into());
        }
        Ok(final_path)
    }

    fn evict_for(&self, incoming: u64) -> Result<(), ResourceCacheError> {
        let store = self
            .store
            .lock()
            .map_err(|_| ResourceCacheError::Synchronization)?;
        let mut usage = store.document_resource_usage()?;
        for resource in store.unreferenced_document_resources_lru()? {
            if usage.saturating_add(incoming) <= self.maximum_bytes {
                break;
            }
            let path = self.root.join(&resource.relative_path);
            if !store.claim_document_resource_eviction(&resource.digest)? {
                continue;
            }
            match fs::remove_file(path) {
                Ok(()) => {
                    store.finish_document_resource_eviction(&resource.digest)?;
                    usage = usage.saturating_sub(resource.byte_length);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    store.finish_document_resource_eviction(&resource.digest)?;
                    usage = usage.saturating_sub(resource.byte_length);
                }
                Err(error) => {
                    store.cancel_document_resource_eviction(&resource.digest)?;
                    drop(store);
                    return Err(error.into());
                }
            }
        }
        drop(store);
        if usage.saturating_add(incoming) > self.maximum_bytes {
            return Err(ResourceCacheError::Limit);
        }
        Ok(())
    }

    fn reconcile(&self) -> Result<(), ResourceCacheError> {
        let store = self
            .store
            .lock()
            .map_err(|_| ResourceCacheError::Synchronization)?;
        for resource in store.unreferenced_document_resources_lru()? {
            let path = self.root.join(&resource.relative_path);
            let descriptor = NodeResourceDescriptor {
                digest: resource.digest.clone(),
                media_type: resource.media_type,
                byte_length: resource.byte_length,
            };
            if !verify_file(&path, &descriptor)? {
                if !store.claim_document_resource_eviction(&resource.digest)? {
                    continue;
                }
                match fs::remove_file(path) {
                    Ok(()) => {
                        store.finish_document_resource_eviction(&resource.digest)?;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        store.finish_document_resource_eviction(&resource.digest)?;
                    }
                    Err(error) => {
                        store.cancel_document_resource_eviction(&resource.digest)?;
                        return Err(error.into());
                    }
                }
            }
        }
        drop(store);
        self.evict_for(0)
    }
}

fn validate_descriptor(
    descriptor: &NodeResourceDescriptor,
    maximum_resource_bytes: u64,
) -> Result<(), ResourceCacheError> {
    if descriptor.digest.len() != 64
        || !descriptor
            .digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || descriptor.media_type.is_empty()
        || descriptor.byte_length > maximum_resource_bytes
    {
        return Err(ResourceCacheError::InvalidDescriptor);
    }
    Ok(())
}

fn relative_path(digest: &str) -> PathBuf {
    PathBuf::from("sha256")
        .join(&digest[..2])
        .join(digest.to_ascii_lowercase())
}

fn verify_file(
    path: &Path,
    descriptor: &NodeResourceDescriptor,
) -> Result<bool, ResourceCacheError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    Ok(
        u64::try_from(bytes.len()).unwrap_or(u64::MAX) == descriptor.byte_length
            && format!("{:x}", Sha256::digest(bytes)) == descriptor.digest.to_ascii_lowercase(),
    )
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "test fixtures should fail at their exact assertion"
)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;
    use tempfile::TempDir;

    fn descriptor(bytes: &[u8], media_type: &str) -> NodeResourceDescriptor {
        NodeResourceDescriptor {
            digest: format!("{:x}", Sha256::digest(bytes)),
            media_type: media_type.into(),
            byte_length: bytes.len() as u64,
        }
    }

    fn cache(temp: &TempDir, maximum: u64) -> DocumentResourceCache {
        DocumentResourceCache::open(
            temp.path().join("assets"),
            temp.path().join("agent.db"),
            maximum,
            maximum,
        )
        .unwrap()
    }

    #[test]
    fn survives_restart_and_rejects_corruption() {
        let temp = TempDir::new().unwrap();
        let bytes = b"verified logo";
        let descriptor = descriptor(bytes, "image/jpeg");
        let path = cache(&temp, 1024)
            .resolve(&descriptor, || Ok(std::io::Cursor::new(bytes)), 1)
            .unwrap();
        drop(cache(&temp, 1024));
        assert_eq!(
            cache(&temp, 1024)
                .resolve(
                    &descriptor,
                    || Ok(std::io::Cursor::new(Vec::<u8>::new())),
                    2
                )
                .unwrap(),
            path
        );
        fs::write(&path, b"corrupt").unwrap();
        let repaired = cache(&temp, 1024)
            .resolve(&descriptor, || Ok(std::io::Cursor::new(bytes)), 3)
            .unwrap();
        assert_eq!(fs::read(repaired).unwrap(), bytes);
    }

    #[test]
    fn collapses_concurrent_acquisition() {
        let temp = TempDir::new().unwrap();
        let cache = Arc::new(cache(&temp, 1024));
        let descriptor = descriptor(b"shared", "image/jpeg");
        let calls = Arc::new(AtomicUsize::new(0));
        let threads: Vec<_> = (0..16)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let descriptor = descriptor.clone();
                let calls = Arc::clone(&calls);
                thread::spawn(move || {
                    cache
                        .resolve(
                            &descriptor,
                            || {
                                calls.fetch_add(1, Ordering::SeqCst);
                                thread::sleep(std::time::Duration::from_millis(20));
                                Ok(std::io::Cursor::new(b"shared"))
                            },
                            1,
                        )
                        .unwrap()
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn evicts_lru_but_never_pinned_resources() {
        let temp = TempDir::new().unwrap();
        let cache = cache(&temp, 8);
        let first = descriptor(b"aaaa", "image/jpeg");
        let second = descriptor(b"bbbb", "image/jpeg");
        let third = descriptor(b"cccc", "image/jpeg");
        cache
            .resolve(&first, || Ok(std::io::Cursor::new(b"aaaa")), 1)
            .unwrap();
        cache.pin(&first.digest).unwrap();
        cache
            .resolve(&second, || Ok(std::io::Cursor::new(b"bbbb")), 2)
            .unwrap();
        cache
            .resolve(&third, || Ok(std::io::Cursor::new(b"cccc")), 3)
            .unwrap();
        assert!(
            cache
                .resolve(&first, || Ok(std::io::Cursor::new(Vec::<u8>::new())), 4)
                .is_ok()
        );
        let calls = AtomicUsize::new(0);
        cache
            .resolve(
                &second,
                || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(std::io::Cursor::new(b"bbbb"))
                },
                5,
            )
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn oversized_stream_is_bounded_and_rejected() {
        let temp = TempDir::new().unwrap();
        let cache = cache(&temp, 4);
        let descriptor = descriptor(b"good", "image/jpeg");
        let error = cache
            .resolve(
                &descriptor,
                || Ok(std::io::Cursor::new(vec![0_u8; 1024 * 1024])),
                1,
            )
            .unwrap_err();
        assert!(matches!(error, ResourceCacheError::Acquisition(_)));
    }

    #[test]
    fn uppercase_digest_is_rejected_as_noncanonical() {
        let temp = TempDir::new().unwrap();
        let cache = cache(&temp, 64);
        let mut descriptor = descriptor(b"data", "image/jpeg");
        descriptor.digest.make_ascii_uppercase();
        assert!(matches!(
            cache.resolve(&descriptor, || Ok(std::io::Cursor::new(b"data")), 1),
            Err(ResourceCacheError::InvalidDescriptor)
        ));
    }

    #[test]
    fn inventory_is_bounded_verified_and_restart_clears_stale_pins() {
        let temp = TempDir::new().unwrap();
        let first = descriptor(b"1111", "image/jpeg");
        let first_path = {
            let cache = cache(&temp, 8);
            let path = cache
                .resolve(&first, || Ok(std::io::Cursor::new(b"1111")), 1)
                .unwrap();
            cache.pin(&first.digest).unwrap();
            assert_eq!(cache.verified_digests(256).unwrap(), vec![first.digest]);
            path
        };
        let cache = cache(&temp, 4);
        let second = descriptor(b"2222", "image/jpeg");
        cache
            .resolve(&second, || Ok(std::io::Cursor::new(b"2222")), 2)
            .unwrap();
        assert!(
            !first_path.exists(),
            "stale crash pin must not leak capacity"
        );
        assert_eq!(cache.verified_digests(1).unwrap(), vec![second.digest]);
    }
}
