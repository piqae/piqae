//! Content-addressed object storage used by the hosted and self-hosted control plane.

use apache_object_store::{
    ObjectStore as ApacheObjectStore, aws::AmazonS3Builder, path::Path as ObjectPath,
};
use async_trait::async_trait;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use thiserror::Error;
use tokio::sync::RwLock;

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
        let temporary = path.with_extension("spool-part");
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
        let root = std::env::temp_dir().join(format!("spool-object-store-{}", std::process::id()));
        let store = FileObjectStore::new(root).await.unwrap();
        assert!(matches!(
            store.put("../outside", Bytes::new(), None).await,
            Err(ObjectStoreError::InvalidKey)
        ));
    }
}
