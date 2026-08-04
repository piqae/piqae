//! Immutable, provider-neutral usage records.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageKind {
    /// Immutable pre-cutover records retained for billing audit continuity.
    PrintJobAccepted,
    PrintJobReportedComplete,
    Adjustment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageEntry {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub environment_id: Uuid,
    pub job_id: Option<Uuid>,
    pub kind: UsageKind,
    pub units: i64,
    pub occurred_at: OffsetDateTime,
    pub reason: Option<String>,
}

#[derive(Debug, Default)]
pub struct UsageLedger {
    entries: Vec<UsageEntry>,
    billed_jobs: BTreeSet<Uuid>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UsageError {
    #[error("live print usage requires a job")]
    MissingJob,
    #[error("adjustments require a non-empty reason")]
    MissingReason,
}

impl UsageLedger {
    /// Records one unit the first time a live job is reported complete.
    /// Returns `None` for test jobs and duplicate completion events.
    pub fn record_reported_completion(
        &mut self,
        workspace_id: Uuid,
        environment_id: Uuid,
        job_id: Uuid,
        is_live: bool,
        occurred_at: OffsetDateTime,
    ) -> Option<&UsageEntry> {
        if !is_live || !self.billed_jobs.insert(job_id) {
            return None;
        }
        self.entries.push(UsageEntry {
            id: Uuid::now_v7(),
            workspace_id,
            environment_id,
            job_id: Some(job_id),
            kind: UsageKind::PrintJobReportedComplete,
            units: 1,
            occurred_at,
            reason: None,
        });
        self.entries.last()
    }

    pub fn adjust(
        &mut self,
        workspace_id: Uuid,
        environment_id: Uuid,
        units: i64,
        reason: impl Into<String>,
        occurred_at: OffsetDateTime,
    ) -> Result<&UsageEntry, UsageError> {
        let reason = reason.into();
        if reason.trim().is_empty() {
            return Err(UsageError::MissingReason);
        }
        self.entries.push(UsageEntry {
            id: Uuid::now_v7(),
            workspace_id,
            environment_id,
            job_id: None,
            kind: UsageKind::Adjustment,
            units,
            occurred_at,
            reason: Some(reason),
        });
        Ok(self.entries.last().expect("entry was just inserted"))
    }

    pub fn entries(&self) -> &[UsageEntry] {
        &self.entries
    }

    pub fn total_units(&self) -> i64 {
        self.entries.iter().map(|entry| entry.units).sum()
    }
}

pub trait BillingProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn automatic_charging_enabled(&self) -> bool;
}

#[derive(Debug, Default)]
pub struct ManualBillingProvider;

impl BillingProvider for ManualBillingProvider {
    fn name(&self) -> &'static str {
        "manual"
    }

    fn automatic_charging_enabled(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reported_completion_is_live_only_and_idempotent() {
        let mut ledger = UsageLedger::default();
        let workspace = Uuid::now_v7();
        let environment = Uuid::now_v7();
        let job = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        assert!(
            ledger
                .record_reported_completion(workspace, environment, job, false, now)
                .is_none()
        );
        assert!(
            ledger
                .record_reported_completion(workspace, environment, job, true, now)
                .is_some()
        );
        assert!(
            ledger
                .record_reported_completion(workspace, environment, job, true, now)
                .is_none()
        );
        assert_eq!(ledger.total_units(), 1);
    }
}
