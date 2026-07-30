//! Content-addressed object storage used by the hosted and self-hosted control plane.

use apache_object_store::{
    ObjectStore as ApacheObjectStore, ObjectStoreExt, aws::AmazonS3Builder,
    gcp::GoogleCloudStorageBuilder, path::Path as ObjectPath,
};
use async_trait::async_trait;
use bytes::Bytes;
use futures::{Stream, StreamExt, stream};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::PathBuf, pin::Pin, sync::Arc};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock;
use tokio_util::io::ReaderStream;

pub type ObjectByteStream =
    Pin<Box<dyn Stream<Item = Result<Bytes, ObjectStoreError>> + Send + 'static>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    pub key: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Error)]
pub enum ObjectStoreError {
    #[error("object not found")]
    NotFound,
    #[error("object digest did not match the supplied digest")]
    DigestMismatch,
    #[error("object length did not match the supplied length")]
    LengthMismatch,
    #[error("object byte stream failed: {0}")]
    Stream(String),
    #[error("invalid object key")]
    InvalidKey,
    #[error("object store I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("S3-compatible object operation failed: {0}")]
    S3(String),
}

#[derive(Debug, Clone)]
pub struct S3ObjectStore {
    inner: Arc<dyn ApacheObjectStore>,
}

#[derive(Debug, Clone)]
pub struct S3Configuration {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub allow_http: bool,
    pub virtual_hosted_style: bool,
}

pub type GcsObjectStore = S3ObjectStore;

#[derive(Debug, Clone)]
pub struct GcsConfiguration {
    pub bucket: String,
    pub service_account_path: Option<String>,
}

impl S3ObjectStore {
    pub fn new(configuration: S3Configuration) -> Result<Self, ObjectStoreError> {
        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(configuration.bucket)
            .with_region(configuration.region)
            .with_access_key_id(configuration.access_key_id)
            .with_secret_access_key(configuration.secret_access_key)
            .with_allow_http(configuration.allow_http)
            .with_virtual_hosted_style_request(configuration.virtual_hosted_style);
        if let Some(endpoint) = configuration.endpoint {
            builder = builder.with_endpoint(endpoint);
        }
        let inner = builder
            .build()
            .map_err(|error| ObjectStoreError::S3(error.to_string()))?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }

    /// Build a native GCS JSON API client. With no explicit key path the
    /// underlying client uses Application Default Credentials, including the
    /// Cloud Run metadata identity.
    pub fn new_gcs(configuration: GcsConfiguration) -> Result<Self, ObjectStoreError> {
        let mut builder =
            GoogleCloudStorageBuilder::from_env().with_bucket_name(configuration.bucket);
        if let Some(path) = configuration.service_account_path {
            builder = builder.with_service_account_path(path);
        }
        let inner = builder
            .build()
            .map_err(|error| ObjectStoreError::S3(error.to_string()))?;
        Ok(Self {
            inner: Arc::new(inner),
        })
    }
}

#[cfg(test)]
mod gcs_configuration_tests {
    use super::{GcsConfiguration, GcsObjectStore};

    #[test]
    fn gcs_builder_accepts_application_default_credentials() {
        let store = GcsObjectStore::new_gcs(GcsConfiguration {
            bucket: "piqae-test-bucket".into(),
            service_account_path: None,
        });
        assert!(store.is_ok());
    }
}

#[async_trait]
impl ObjectStore for S3ObjectStore {
    async fn put(
        &self,
        key: &str,
        content: Bytes,
        expected_sha256: Option<&str>,
    ) -> Result<StoredObject, ObjectStoreError> {
        validate_key(key)?;
        let sha256 = digest_hex(&content);
        if expected_sha256.is_some_and(|expected| !expected.eq_ignore_ascii_case(&sha256)) {
            return Err(ObjectStoreError::DigestMismatch);
        }
        let bytes = content.len() as u64;
        self.inner
            .put(&ObjectPath::from(key), content.into())
            .await
            .map_err(|error| ObjectStoreError::S3(error.to_string()))?;
        Ok(StoredObject {
            key: key.to_owned(),
            sha256,
            bytes,
        })
    }

    async fn get(&self, key: &str) -> Result<Bytes, ObjectStoreError> {
        validate_key(key)?;
        self.inner
            .get(&ObjectPath::from(key))
            .await
            .map_err(|error| ObjectStoreError::S3(error.to_string()))?
            .bytes()
            .await
            .map_err(|error| ObjectStoreError::S3(error.to_string()))
    }

    async fn put_stream(
        &self,
        key: &str,
        mut content: ObjectByteStream,
        expected_sha256: &str,
        expected_bytes: u64,
    ) -> Result<StoredObject, ObjectStoreError> {
        validate_key(key)?;
        let mut writer = apache_object_store::buffered::BufWriter::new(
            Arc::clone(&self.inner),
            ObjectPath::from(key),
        );
        let mut hasher = Sha256::new();
        let mut bytes = 0_u64;
        while let Some(chunk) = content.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    let _ = writer.abort().await;
                    return Err(error);
                }
            };
            bytes = bytes
                .checked_add(chunk.len() as u64)
                .ok_or(ObjectStoreError::LengthMismatch)?;
            if bytes > expected_bytes {
                let _ = writer.abort().await;
                return Err(ObjectStoreError::LengthMismatch);
            }
            hasher.update(&chunk);
            if let Err(error) = writer.put(chunk).await {
                let _ = writer.abort().await;
                return Err(ObjectStoreError::S3(error.to_string()));
            }
        }
        let sha256 = format!("{:x}", hasher.finalize());
        if bytes != expected_bytes {
            let _ = writer.abort().await;
            return Err(ObjectStoreError::LengthMismatch);
        }
        if !expected_sha256.eq_ignore_ascii_case(&sha256) {
            let _ = writer.abort().await;
            return Err(ObjectStoreError::DigestMismatch);
        }
        writer
            .shutdown()
            .await
            .map_err(|error| ObjectStoreError::S3(error.to_string()))?;
        Ok(StoredObject {
            key: key.to_owned(),
            sha256,
            bytes,
        })
    }

    async fn get_stream(&self, key: &str) -> Result<ObjectByteStream, ObjectStoreError> {
        validate_key(key)?;
        let stream = self
            .inner
            .get(&ObjectPath::from(key))
            .await
            .map_err(|error| ObjectStoreError::S3(error.to_string()))?
            .into_stream()
            .map(|result| result.map_err(|error| ObjectStoreError::S3(error.to_string())));
        Ok(Box::pin(stream))
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        validate_key(key)?;
        self.inner
            .delete(&ObjectPath::from(key))
            .await
            .map_err(|error| ObjectStoreError::S3(error.to_string()))
    }

    async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError> {
        validate_key(key)?;
        match self.inner.head(&ObjectPath::from(key)).await {
            Ok(_) => Ok(true),
            Err(apache_object_store::Error::NotFound { .. }) => Ok(false),
            Err(error) => Err(ObjectStoreError::S3(error.to_string())),
        }
    }
}

#[async_trait]
pub trait ObjectStore: Send + Sync + 'static {
    async fn put(
        &self,
        key: &str,
        content: Bytes,
        expected_sha256: Option<&str>,
    ) -> Result<StoredObject, ObjectStoreError>;
    async fn get(&self, key: &str) -> Result<Bytes, ObjectStoreError>;
    async fn put_stream(
        &self,
        key: &str,
        content: ObjectByteStream,
        expected_sha256: &str,
        expected_bytes: u64,
    ) -> Result<StoredObject, ObjectStoreError>;
    async fn get_stream(&self, key: &str) -> Result<ObjectByteStream, ObjectStoreError>;
    async fn verify(
        &self,
        key: &str,
        expected_sha256: &str,
        expected_bytes: u64,
    ) -> Result<StoredObject, ObjectStoreError> {
        let content = self.get_stream(key).await?;
        verify_stream(key, content, expected_sha256, expected_bytes).await
    }
    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError>;
    async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError>;
}

pub fn digest_hex(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}

fn validate_key(key: &str) -> Result<(), ObjectStoreError> {
    let valid = !key.is_empty()
        && !key.starts_with('/')
        && !key.contains('\\')
        && key.split('/').all(|part| !matches!(part, "" | "." | ".."));
    valid.then_some(()).ok_or(ObjectStoreError::InvalidKey)
}

#[derive(Debug, Clone, Default)]
pub struct MemoryObjectStore {
    objects: Arc<RwLock<HashMap<String, Bytes>>>,
}

#[async_trait]
impl ObjectStore for MemoryObjectStore {
    async fn put(
        &self,
        key: &str,
        content: Bytes,
        expected_sha256: Option<&str>,
    ) -> Result<StoredObject, ObjectStoreError> {
        validate_key(key)?;
        let sha256 = digest_hex(&content);
        if expected_sha256.is_some_and(|expected| !expected.eq_ignore_ascii_case(&sha256)) {
            return Err(ObjectStoreError::DigestMismatch);
        }
        let bytes = content.len() as u64;
        self.objects.write().await.insert(key.to_owned(), content);
        Ok(StoredObject {
            key: key.to_owned(),
            sha256,
            bytes,
        })
    }

    async fn get(&self, key: &str) -> Result<Bytes, ObjectStoreError> {
        validate_key(key)?;
        self.objects
            .read()
            .await
            .get(key)
            .cloned()
            .ok_or(ObjectStoreError::NotFound)
    }

    async fn put_stream(
        &self,
        key: &str,
        content: ObjectByteStream,
        expected_sha256: &str,
        expected_bytes: u64,
    ) -> Result<StoredObject, ObjectStoreError> {
        let (object, chunks) =
            collect_verified_stream(key, content, expected_sha256, expected_bytes).await?;
        self.objects
            .write()
            .await
            .insert(key.to_owned(), chunks.concat().into());
        Ok(object)
    }

    async fn get_stream(&self, key: &str) -> Result<ObjectByteStream, ObjectStoreError> {
        let content = self.get(key).await?;
        Ok(Box::pin(stream::once(async move { Ok(content) })))
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        validate_key(key)?;
        self.objects.write().await.remove(key);
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError> {
        validate_key(key)?;
        Ok(self.objects.read().await.contains_key(key))
    }
}

#[derive(Debug, Clone)]
pub struct FileObjectStore {
    root: Arc<PathBuf>,
}

impl FileObjectStore {
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, ObjectStoreError> {
        let root = root.into();
        tokio::fs::create_dir_all(&root).await?;
        Ok(Self {
            root: Arc::new(root),
        })
    }

    fn path_for(&self, key: &str) -> Result<PathBuf, ObjectStoreError> {
        validate_key(key)?;
        Ok(self.root.join(key))
    }
}

#[async_trait]
impl ObjectStore for FileObjectStore {
    async fn put(
        &self,
        key: &str,
        content: Bytes,
        expected_sha256: Option<&str>,
    ) -> Result<StoredObject, ObjectStoreError> {
        let path = self.path_for(key)?;
        let sha256 = digest_hex(&content);
        if expected_sha256.is_some_and(|expected| !expected.eq_ignore_ascii_case(&sha256)) {
            return Err(ObjectStoreError::DigestMismatch);
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let temporary = path.with_extension("piqae-part");
        tokio::fs::write(&temporary, &content).await?;
        tokio::fs::rename(&temporary, &path).await?;
        Ok(StoredObject {
            key: key.to_owned(),
            sha256,
            bytes: content.len() as u64,
        })
    }

    async fn get(&self, key: &str) -> Result<Bytes, ObjectStoreError> {
        let path = self.path_for(key)?;
        match tokio::fs::read(path).await {
            Ok(content) => Ok(Bytes::from(content)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err(ObjectStoreError::NotFound)
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn put_stream(
        &self,
        key: &str,
        mut content: ObjectByteStream,
        expected_sha256: &str,
        expected_bytes: u64,
    ) -> Result<StoredObject, ObjectStoreError> {
        let path = self.path_for(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let temporary = path.with_extension(format!("{}.piqae-part", ulid_fragment()));
        let mut output = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .await?;
        let result = async {
            let mut hasher = Sha256::new();
            let mut bytes = 0_u64;
            while let Some(chunk) = content.next().await {
                let chunk = chunk?;
                bytes = bytes
                    .checked_add(chunk.len() as u64)
                    .ok_or(ObjectStoreError::LengthMismatch)?;
                if bytes > expected_bytes {
                    return Err(ObjectStoreError::LengthMismatch);
                }
                hasher.update(&chunk);
                output.write_all(&chunk).await?;
            }
            output.flush().await?;
            let sha256 = format!("{:x}", hasher.finalize());
            if bytes != expected_bytes {
                return Err(ObjectStoreError::LengthMismatch);
            }
            if !expected_sha256.eq_ignore_ascii_case(&sha256) {
                return Err(ObjectStoreError::DigestMismatch);
            }
            Ok(StoredObject {
                key: key.to_owned(),
                sha256,
                bytes,
            })
        }
        .await;
        drop(output);
        match result {
            Ok(object) => {
                tokio::fs::rename(&temporary, &path).await?;
                Ok(object)
            }
            Err(error) => {
                let _ = tokio::fs::remove_file(&temporary).await;
                Err(error)
            }
        }
    }

    async fn get_stream(&self, key: &str) -> Result<ObjectByteStream, ObjectStoreError> {
        let path = self.path_for(key)?;
        let file = tokio::fs::File::open(path).await.map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ObjectStoreError::NotFound
            } else {
                ObjectStoreError::Io(error)
            }
        })?;
        Ok(Box::pin(
            ReaderStream::new(file).map(|result| result.map_err(ObjectStoreError::Io)),
        ))
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        let path = self.path_for(key)?;
        match tokio::fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError> {
        Ok(tokio::fs::try_exists(self.path_for(key)?).await?)
    }
}

async fn verify_stream(
    key: &str,
    mut content: ObjectByteStream,
    expected_sha256: &str,
    expected_bytes: u64,
) -> Result<StoredObject, ObjectStoreError> {
    validate_key(key)?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    while let Some(chunk) = content.next().await {
        let chunk = chunk?;
        bytes = bytes
            .checked_add(chunk.len() as u64)
            .ok_or(ObjectStoreError::LengthMismatch)?;
        if bytes > expected_bytes {
            return Err(ObjectStoreError::LengthMismatch);
        }
        hasher.update(&chunk);
    }
    let sha256 = format!("{:x}", hasher.finalize());
    if bytes != expected_bytes {
        return Err(ObjectStoreError::LengthMismatch);
    }
    if !expected_sha256.eq_ignore_ascii_case(&sha256) {
        return Err(ObjectStoreError::DigestMismatch);
    }
    Ok(StoredObject {
        key: key.to_owned(),
        sha256,
        bytes,
    })
}

async fn collect_verified_stream(
    key: &str,
    mut content: ObjectByteStream,
    expected_sha256: &str,
    expected_bytes: u64,
) -> Result<(StoredObject, Vec<Bytes>), ObjectStoreError> {
    let mut chunks = Vec::new();
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    while let Some(chunk) = content.next().await {
        let chunk = chunk?;
        bytes = bytes
            .checked_add(chunk.len() as u64)
            .ok_or(ObjectStoreError::LengthMismatch)?;
        if bytes > expected_bytes {
            return Err(ObjectStoreError::LengthMismatch);
        }
        hasher.update(&chunk);
        chunks.push(chunk);
    }
    let sha256 = format!("{:x}", hasher.finalize());
    if bytes != expected_bytes {
        return Err(ObjectStoreError::LengthMismatch);
    }
    if !expected_sha256.eq_ignore_ascii_case(&sha256) {
        return Err(ObjectStoreError::DigestMismatch);
    }
    Ok((
        StoredObject {
            key: key.to_owned(),
            sha256,
            bytes,
        },
        chunks,
    ))
}

fn ulid_fragment() -> String {
    format!("{}-{}", std::process::id(), chrono_like_timestamp())
}

fn chrono_like_timestamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memory_store_checks_digest_and_round_trips() {
        let store = MemoryObjectStore::default();
        let data = Bytes::from_static(b"print me");
        let digest = digest_hex(&data);
        let object = store
            .put("workspace/content", data.clone(), Some(&digest))
            .await
            .unwrap();
        assert_eq!(object.bytes, 8);
        assert_eq!(store.get("workspace/content").await.unwrap(), data);
        assert!(
            store
                .put("bad", Bytes::from_static(b"x"), Some("nope"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn keys_cannot_escape_filesystem_root() {
        let root = std::env::temp_dir().join(format!("piqae-object-store-{}", std::process::id()));
        let store = FileObjectStore::new(root).await.unwrap();
        assert!(matches!(
            store.put("../outside", Bytes::new(), None).await,
            Err(ObjectStoreError::InvalidKey)
        ));
    }

    #[tokio::test]
    async fn streamed_put_is_bounded_digest_checked_and_readable() {
        let store = MemoryObjectStore::default();
        let expected = Bytes::from_static(b"large-document");
        let digest = digest_hex(&expected);
        let chunks: ObjectByteStream = Box::pin(stream::iter([
            Ok(Bytes::from_static(b"large-")),
            Ok(Bytes::from_static(b"document")),
        ]));
        let stored = store
            .put_stream("workspace/streamed", chunks, &digest, 14)
            .await
            .expect("valid bounded stream");
        assert_eq!(stored.bytes, 14);
        assert_eq!(
            store
                .get("workspace/streamed")
                .await
                .expect("stored object"),
            expected
        );

        let oversized: ObjectByteStream =
            Box::pin(stream::once(async { Ok(Bytes::from_static(b"too-long")) }));
        assert!(matches!(
            store
                .put_stream("workspace/rejected", oversized, &digest, 2)
                .await,
            Err(ObjectStoreError::LengthMismatch)
        ));
        assert!(!store.exists("workspace/rejected").await.expect("exists"));
    }
}
