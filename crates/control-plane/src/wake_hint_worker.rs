//! Durable provider-neutral dispatch for content-free node wake hints.

use crate::{AppState, authentication::TenantContext, destination_topology::wake_hint_response};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct WakeHintWorker {
    state: AppState,
}

impl WakeHintWorker {
    #[must_use]
    pub const fn new(state: AppState) -> Self {
        Self { state }
    }

    /// Repairs N-1 waiting transitions, then publishes claimed hints into the
    /// signed tenant webhook stream. Publication is at-least-once: a crash
    /// after `publish` but before completion retries the same opaque hint ID.
    ///
    /// # Errors
    ///
    /// Returns a repository error when reconciliation, claiming, or an outbox
    /// state transition fails. Publication failures are durably rescheduled
    /// and suppressed so one unavailable webhook cannot block the batch.
    pub async fn run_once(&self, limit: i64) -> Result<usize, crate::repository::RepositoryError> {
        self.state
            .repository
            .repair_waiting_job_wake_hints(limit)
            .await?;
        let work = self
            .state
            .repository
            .claim_wake_hint_dispatches(limit)
            .await?;
        let mut completed = 0_usize;
        for item in work {
            let tenant = TenantContext::unrestricted(item.workspace_id, item.environment_id);
            let ensure = self
                .state
                .destination_topology
                .create_node_wake_hint(
                    crate::destination_topology::tenant_scope(tenant),
                    &item.hint,
                    &item.idempotency_key,
                )
                .await;
            let published = match ensure {
                Ok(hint) if hint.status == "pending" && hint.expires_at > chrono::Utc::now() => {
                    self.state
                        .publish(
                            tenant,
                            "node.wake_hint.requested",
                            &wake_hint_response(hint),
                        )
                        .await
                }
                // Observation, cancellation, or expiry won before dispatch.
                // Completing the stale outbox item emits no wake event.
                Ok(_) => Ok(()),
                Err(error) => Err(crate::repository::RepositoryError::Persistence(
                    error.to_string(),
                )),
            };
            if published.is_ok() {
                self.state
                    .repository
                    .complete_wake_hint_dispatch(&item.outbox_id)
                    .await?;
                completed = completed.saturating_add(1);
            } else {
                let shift = item.attempt.saturating_sub(1).min(8);
                let delay = Duration::from_secs((1_u64 << shift).min(300));
                self.state
                    .repository
                    .retry_wake_hint_dispatch(&item.outbox_id, delay)
                    .await?;
            }
        }
        Ok(completed)
    }
}
