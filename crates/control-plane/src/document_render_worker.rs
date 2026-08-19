//! Durable, lease-based document rendering.

use crate::{
    AppState,
    documents::{artifact_key_aad, document_aad, render_input_aad},
    repository::RepositoryError,
};
use bytes::Bytes;
use piqae_document_renderer::{
    BusinessDocumentV1, RenderLimits, ResolvedResources, render_with_metrics,
};
use piqae_storage_postgres::DocumentRenderWork;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

#[derive(Clone, Debug)]
pub struct DocumentRenderWorker {
    state: AppState,
    worker_id: Arc<str>,
    concurrency: usize,
    render_permits: Arc<Semaphore>,
    timeout: Duration,
    lease_seconds: i64,
}

impl DocumentRenderWorker {
    #[must_use]
    pub fn new(state: AppState, worker_id: impl Into<Arc<str>>) -> Self {
        Self {
            state,
            worker_id: worker_id.into(),
            concurrency: 4,
            render_permits: Arc::new(Semaphore::new(4)),
            timeout: Duration::from_secs(20),
            lease_seconds: 60,
        }
    }

    #[must_use]
    pub fn with_limits(
        mut self,
        concurrency: usize,
        timeout: Duration,
        lease_seconds: i64,
    ) -> Self {
        self.concurrency = concurrency;
        self.render_permits = Arc::new(Semaphore::new(concurrency.clamp(1, 32)));
        self.timeout = timeout;
        self.lease_seconds = lease_seconds;
        self
    }

    /// Claims and processes at most `limit` records. A lease that expires after
    /// process death becomes claimable by another instance on its next poll.
    ///
    /// # Errors
    /// Returns a repository error when work cannot be claimed.
    pub async fn run_once(&self, limit: i64) -> Result<usize, RepositoryError> {
        let capacity = self.concurrency.clamp(1, 32);
        let work = self
            .state
            .repository
            .claim_document_renders(
                &self.worker_id,
                limit.min(i64::try_from(capacity).unwrap_or(32)),
                self.lease_seconds,
            )
            .await?;
        let count = work.len();
        let semaphore = Arc::new(Semaphore::new(capacity));
        let mut tasks = tokio::task::JoinSet::new();
        for item in work {
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|error| RepositoryError::Persistence(error.to_string()))?;
            let worker = self.clone();
            tasks.spawn(async move {
                let _permit = permit;
                worker.process(item).await;
            });
        }
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result {
                tracing::error!(error.type="document_render_worker_panic", %error);
            }
        }
        Ok(count)
    }

    /// Deletes expired objects before clearing their encrypted references. A
    /// failed object-store delete leaves the leased row recoverable after TTL.
    ///
    /// # Errors
    /// Returns a repository error when cleanup cannot be claimed or finalized.
    pub async fn cleanup_once(&self, limit: i64) -> Result<usize, RepositoryError> {
        let work = self
            .state
            .repository
            .claim_expired_document_artifacts(&self.worker_id, limit, self.lease_seconds)
            .await?;
        let mut completed = 0;
        for item in work {
            completed += usize::from(self.cleanup_item(&item).await);
        }
        let resources = self
            .state
            .repository
            .claim_expired_business_document_resources(limit)
            .await?;
        for resource in resources {
            let object_key = crate::documents::document_resource_object_key(
                &resource.workspace_id.to_string(),
                &resource.environment_id.to_string(),
                &resource.digest,
            );
            if let Err(error) = self.state.object_store.delete(&object_key).await {
                tracing::warn!(digest=%resource.digest, %error, "resource object expiry will retry");
                continue;
            }
            if let Err(error) = self
                .state
                .repository
                .complete_expired_business_document_resource(&resource)
                .await
            {
                tracing::warn!(digest=%resource.digest, %error, "resource expiry finalization will retry");
                continue;
            }
            completed += 1;
        }
        Ok(completed)
    }

    async fn cleanup_item(
        &self,
        item: &piqae_storage_postgres::ExpiredDocumentArtifactWork,
    ) -> bool {
        if !self.delete_expired_object(item).await {
            return false;
        }
        if let Err(error) = self
            .state
            .repository
            .complete_document_artifact_expiry(item)
            .await
        {
            tracing::warn!(render_id=%item.render_id, %error, "artifact expiry finalization will retry");
            return false;
        }
        true
    }

    async fn delete_expired_object(
        &self,
        item: &piqae_storage_postgres::ExpiredDocumentArtifactWork,
    ) -> bool {
        if let Some(ciphertext) = &item.object_key_ciphertext {
            let aad = artifact_key_aad(
                &item.workspace_id.to_string(),
                &item.environment_id.to_string(),
                &item.render_id,
            );
            let Some(key) = self
                .state
                .document_secrets
                .decrypt(&aad, ciphertext)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
            else {
                tracing::error!(render_id=%item.render_id, "expired artifact key could not be decrypted");
                return false;
            };
            if let Err(error) = self.state.object_store.delete(&key).await {
                tracing::warn!(render_id=%item.render_id, %error, "expired artifact deletion will retry");
                return false;
            }
        }
        true
    }

    async fn process(&self, work: DocumentRenderWork) {
        if let Err((code, retryable)) = self.render_and_complete(&work).await {
            if let Err(error) = self
                .state
                .repository
                .fail_claimed_document_render(&work, code, retryable)
                .await
            {
                tracing::warn!(render_id=%work.render.id, %error, "could not record document render failure");
            }
        }
    }

    async fn render_and_complete(
        &self,
        work: &DocumentRenderWork,
    ) -> Result<(), (&'static str, bool)> {
        let revision = self
            .state
            .repository
            .get_document_revision(
                work.workspace_id,
                work.environment_id,
                &work.render.template_revision_id,
            )
            .await
            .map_err(|_| ("revision_unavailable", true))?;
        let revision_aad = document_aad(
            &work.workspace_id.to_string(),
            &work.environment_id.to_string(),
            &revision.template_id,
        );
        let spec_bytes = self
            .state
            .document_secrets
            .decrypt(&revision_aad, &revision.spec_ciphertext)
            .map_err(|_| ("document_decryption_failed", false))?;
        let input_aad = render_input_aad(
            &work.workspace_id.to_string(),
            &work.environment_id.to_string(),
            &work.render.id,
        );
        let input_bytes = self
            .state
            .document_secrets
            .decrypt(&input_aad, &work.render.input_ciphertext)
            .map_err(|_| ("document_decryption_failed", false))?;
        let spec: BusinessDocumentV1 =
            serde_json::from_slice(&spec_bytes).map_err(|_| ("invalid_document_spec", false))?;
        let input: Value =
            serde_json::from_slice(&input_bytes).map_err(|_| ("invalid_document_input", false))?;
        // A timed-out blocking task keeps running. Holding this permit inside
        // the closure bounds those stragglers across worker polling cycles.
        let permit =
            tokio::time::timeout(self.timeout, self.render_permits.clone().acquire_owned())
                .await
                .map_err(|_| ("renderer_capacity_timeout", true))?
                .map_err(|_| ("renderer_unavailable", true))?;
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            render_with_metrics(
                &spec,
                &input,
                &ResolvedResources::default(),
                RenderLimits::default(),
            )
        });
        let output = tokio::time::timeout(self.timeout, task)
            .await
            .map_err(|_| ("render_timeout", true))?
            .map_err(|_| ("render_worker_panic", true))?
            .map_err(|_| ("document_render_failed", false))?;
        let object_key = format!(
            "{}/{}/documents/{}.pdf",
            work.workspace_id, work.environment_id, work.render.id
        );
        let artifact = self
            .state
            .object_store
            .put(&object_key, Bytes::from(output.pdf), None)
            .await
            .map_err(|_| ("document_artifact_store_failed", true))?;
        let encrypted_key = self
            .state
            .document_secrets
            .encrypt(
                &artifact_key_aad(
                    &work.workspace_id.to_string(),
                    &work.environment_id.to_string(),
                    &work.render.id,
                ),
                artifact.key.as_bytes(),
            )
            .map_err(|_| ("document_encryption_failed", true))?;
        let byte_length =
            i64::try_from(artifact.bytes).map_err(|_| ("document_artifact_too_large", false))?;
        self.state
            .repository
            .complete_claimed_document_render(
                work,
                &encrypted_key,
                &artifact.sha256,
                byte_length,
                i32::try_from(output.page_count)
                    .map_err(|_| ("document_page_count_invalid", false))?,
            )
            .await
            .map_err(|_| ("render_lease_lost", true))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        authentication::StaticAuthenticator,
        repository::{MemoryRepository, Repository},
    };
    use piqae_domain::{EnvironmentId, WorkspaceId};
    use piqae_storage_postgres::CreateDocumentResult;
    use sha2::{Digest, Sha256};
    use std::str::FromStr;

    async fn queued() -> Result<
        (
            DocumentRenderWorker,
            Arc<MemoryRepository>,
            WorkspaceId,
            EnvironmentId,
            String,
        ),
        Box<dyn std::error::Error>,
    > {
        let repo = Arc::new(MemoryRepository::default());
        let state = AppState::new_for_tests(repo.clone(), Arc::new(StaticAuthenticator::default()))
            .with_document_key([7; 32]);
        let w = WorkspaceId::from_str("wsp_01J00000000000000000000000")?;
        let e = EnvironmentId::from_str("env_01J00000000000000000000000")?;
        let spec = br#"{"format":"piqae.business-document/v1","media":{"kind":"paged","size":"a4"},"body":[{"type":"paragraph","content":[{"type":"text","value":"Hello"}]}]}"#;
        let tpl_aad = document_aad(&w.to_string(), &e.to_string(), "tpl_test01");
        let enc = state.document_secrets.encrypt(&tpl_aad, spec)?;
        repo.create_document_template(
            w,
            e,
            "tpl_test01",
            "Test",
            &enc,
            &hex::encode(Sha256::digest(spec)),
        )
        .await?;
        repo.publish_document_template(w, e, "tpl_test01", "rev_test01")
            .await?;
        let id = "render_test01".to_owned();
        let input = b"{}";
        let aad = render_input_aad(&w.to_string(), &e.to_string(), &id);
        let input_enc = state.document_secrets.encrypt(&aad, input)?;
        assert!(matches!(
            repo.register_document_render(
                w,
                e,
                &id,
                "rev_test01",
                &input_enc,
                &hex::encode(Sha256::digest(input)),
                "idem_test01",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            )
            .await?,
            CreateDocumentResult::Created(_)
        ));
        Ok((DocumentRenderWorker::new(state, "worker-a"), repo, w, e, id))
    }

    #[tokio::test]
    async fn completes_once_and_cannot_be_reclaimed() -> Result<(), Box<dyn std::error::Error>> {
        let (worker, repo, w, e, id) = queued().await?;
        assert_eq!(worker.run_once(10).await?, 1);
        assert_eq!(
            repo.get_document_render(w, e, &id).await?.state,
            "completed"
        );
        assert_eq!(worker.run_once(10).await?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn expired_lease_is_recovered_and_stale_completion_is_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let (worker, repo, _, _, _) = queued().await?;
        let first = repo
            .claim_document_renders("crashed", 1, 30)
            .await?
            .remove(0);
        // Memory repository's test representation can emulate passage of the lease.
        repo.expire_document_render_lease_for_test(&first.render.id)
            .await;
        let recovered = repo
            .claim_document_renders("replacement", 1, 30)
            .await?
            .remove(0);
        assert_ne!(first.lease_token, recovered.lease_token);
        assert!(matches!(
            repo.complete_claimed_document_render(
                &first,
                b"key",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                10,
                1
            )
            .await,
            Err(RepositoryError::ConcurrentStateChange)
        ));
        assert_eq!(worker.run_once(10).await?, 0); // replacement still owns the live lease
        Ok(())
    }

    #[tokio::test]
    async fn legacy_completion_is_idempotent_only_for_identical_artifact()
    -> Result<(), Box<dyn std::error::Error>> {
        let (_, repo, workspace, environment, id) = queued().await?;
        let digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        repo.complete_document_render(workspace, environment, &id, b"encrypted-key", digest, 10)
            .await?;
        repo.complete_document_render(workspace, environment, &id, b"encrypted-key", digest, 10)
            .await?;
        assert!(matches!(
            repo.complete_document_render(workspace, environment, &id, b"other-key", digest, 10)
                .await,
            Err(RepositoryError::ConcurrentStateChange)
        ));
        Ok(())
    }
}
