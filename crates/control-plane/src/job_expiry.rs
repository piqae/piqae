use crate::{AppState, authentication::TenantContext, repository::RepositoryError};

/// Bounded authoritative expiry for jobs which never crossed the node/native
/// responsibility boundary.
#[derive(Clone, Debug)]
pub struct JobExpiryWorker {
    state: AppState,
}

impl JobExpiryWorker {
    #[must_use]
    pub const fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Expires one bounded batch and broadcasts every already-durable tenant
    /// event to connected subscribers.
    ///
    /// # Errors
    ///
    /// Returns the first persistence/publication error after attempting every
    /// transition in the batch. The idempotent durable outbox remains the
    /// source of truth across worker restart.
    pub async fn run_once(&self, limit: i64) -> Result<usize, RepositoryError> {
        let expired = self
            .state
            .repository
            .expire_jobs_before_handoff(limit)
            .await?;
        let count = expired.len();
        let mut first_error = None;
        for expired in expired {
            let tenant = TenantContext::unrestricted(expired.workspace_id, expired.environment_id);
            if let Err(error) = self
                .state
                .publish_idempotently(
                    &expired.transition.webhook_idempotency_key,
                    tenant,
                    "job.updated",
                    &expired.transition.job,
                )
                .await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(count), Err)
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{
        authentication::StaticAuthenticator,
        repository::{CreateResult, MemoryRepository, Repository},
    };
    use chrono::{Duration, Utc};
    use piqae_domain::{
        AgentId, ContentKind, ContentSource, EnvironmentId, Job, JobId, JobOptions, JobState,
        PrinterId, WorkspaceId,
    };
    use std::{collections::BTreeMap, sync::Arc};

    fn expired_job(
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
        created_at: chrono::DateTime<Utc>,
    ) -> Job {
        Job {
            id: JobId::new(),
            workspace_id,
            environment_id,
            printer_id,
            title: "expiry fixture".into(),
            source: None,
            content_kind: ContentKind::Pdf,
            content: ContentSource::Base64 {
                data: "JVBERi0=".into(),
            },
            options: JobOptions::default(),
            metadata: BTreeMap::new(),
            deliveries: 1,
            state: JobState::WaitingForAgent,
            created_at,
            expires_at: created_at + Duration::seconds(1),
            delivery_uncertain_since: None,
        }
    }

    #[tokio::test]
    async fn memory_expiry_is_bounded_live_durable_and_preserves_local_responsibility() {
        let repository = MemoryRepository::default();
        let workspace_id = WorkspaceId::new();
        let environment_id = EnvironmentId::new();
        let printer_id = PrinterId::new();
        let agent_id = AgentId::new();
        let base = Utc::now() - Duration::minutes(2);
        let mut jobs = Vec::new();
        for offset in 0..4 {
            let job = expired_job(
                workspace_id,
                environment_id,
                printer_id,
                base + Duration::seconds(offset),
            );
            assert!(matches!(
                repository
                    .create_job(&job, agent_id, None, b"expiry")
                    .await
                    .expect("create expired job"),
                CreateResult::Created(_)
            ));
            jobs.push(job);
        }
        repository
            .mark_local_responsibility_for_test(jobs[3].id, agent_id)
            .await;
        let state = AppState::new_for_tests(
            Arc::new(repository.clone()),
            Arc::new(StaticAuthenticator::default()),
        );
        let mut live = state.events.subscribe();
        let worker = JobExpiryWorker::new(state);

        assert_eq!(worker.run_once(2).await.expect("first bounded pass"), 2);
        assert_eq!(worker.run_once(2).await.expect("second bounded pass"), 1);
        assert_eq!(worker.run_once(2).await.expect("idempotent pass"), 0);

        for job in &jobs[..3] {
            assert_eq!(
                repository
                    .get_job(workspace_id, environment_id, job.id)
                    .await
                    .expect("expired job")
                    .state,
                JobState::Expired
            );
            let events = repository
                .list_job_events(workspace_id, environment_id, job.id)
                .await
                .expect("job events");
            assert_eq!(
                events.last().map(|event| event.state),
                Some(JobState::Expired)
            );
        }
        assert_eq!(
            repository
                .get_job(workspace_id, environment_id, jobs[3].id)
                .await
                .expect("accepted job")
                .state,
            JobState::FailedRetryable
        );
        let tenant_events = repository
            .list_tenant_events(workspace_id, environment_id, None, 20)
            .await
            .expect("durable tenant events");
        assert_eq!(
            tenant_events
                .iter()
                .filter(|event| event.event_type == "job.updated")
                .count(),
            3
        );
        for expected in &jobs[..3] {
            let event = live.recv().await.expect("live expiry event");
            assert_eq!(event.event_type, "job.updated");
            assert_eq!(event.data["id"], expected.id.as_ulid().to_string());
            assert_eq!(event.data["state"], "expired");
        }
    }
}
