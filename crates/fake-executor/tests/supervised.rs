#![allow(clippy::expect_used)]

use piqae_agent_core::{Executor as _, LocalSubmission};
use piqae_domain::{ContentKind, JobId, JobOptions};
use piqae_executor_supervisor::{ExecutorSupervisor, SupervisedExecutor};
use piqae_protocol::executor::{
    ExecutorOperation, ExecutorRequest, ExecutorResult, LocalRouteFence, NativeJobState,
};
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;

#[tokio::test]
async fn fake_executor_runs_across_the_framed_process_boundary() {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_piqae-fake-executor"));
    let supervisor = ExecutorSupervisor::new(executable, Duration::from_secs(5));
    let response = supervisor
        .execute(&ExecutorRequest {
            request_id: Uuid::nil(),
            deadline_unix_ms: i64::MAX,
            operation: ExecutorOperation::Submit {
                job_id: JobId::new(),
                native_printer_id: "fake-printer".into(),
                title: "test".into(),
                content_kind: ContentKind::Raw,
                content_path: "/does/not/matter".into(),
                options: JobOptions::default(),
                native_profile: None,
                route_fence: None,
            },
        })
        .await
        .expect("execute");
    assert!(matches!(
        response.result,
        Ok(ExecutorResult::Submitted {
            native_job_id: Some(_),
            route_fence: None,
        })
    ));

    let observed = supervisor
        .execute(&ExecutorRequest {
            request_id: Uuid::new_v4(),
            deadline_unix_ms: i64::MAX,
            operation: ExecutorOperation::Observe {
                native_printer_id: "fake-printer".into(),
                native_job_id: "fake-job-printing".into(),
            },
        })
        .await
        .expect("observe");
    assert!(matches!(
        observed.result,
        Ok(ExecutorResult::Observation { observation })
            if observation.state == NativeJobState::Printing
    ));
}

#[tokio::test]
async fn stable_local_idempotency_job_reaches_the_fake_spooler() {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_piqae-fake-executor"));
    let mut executor =
        SupervisedExecutor::new(ExecutorSupervisor::new(executable, Duration::from_secs(5)));
    let fence = LocalRouteFence {
        route_id: "route-proof-backed".into(),
        reservation_id: Uuid::new_v4(),
        generation: 7,
    };
    let submission = LocalSubmission {
        job_id: "job_local_1b50fe719e9491e92621324d46b437b9".into(),
        submission_id: "local:fixture".into(),
        printer_id: "printer-1".into(),
        printer_native_id: "fake-printer".into(),
        title: "virtual idempotency fixture".into(),
        content_path: PathBuf::from("/does/not/matter"),
        content_kind: "pdf".into(),
        options: JobOptions::default(),
        native_profile: None,
        printer_native_binding: None,
        route_connector_id: Some("legacy".into()),
        deadline_unix_ms: i64::MAX,
        route_fence: Some(fence),
    };
    assert_eq!(submission.route_connector_id.as_deref(), Some("legacy"));
    let accepted = executor
        .submit(submission)
        .await
        .expect("proof-backed owned job reaches virtual process executor");
    assert!(!accepted.native_job_id.is_empty());
}
