#![allow(clippy::expect_used)]

use spool_domain::{ContentKind, JobId, JobOptions};
use spool_executor_supervisor::ExecutorSupervisor;
use spool_protocol::executor::{
    ExecutorOperation, ExecutorRequest, ExecutorResult, NativeJobState,
};
use std::{path::PathBuf, time::Duration};
use uuid::Uuid;

#[tokio::test]
async fn fake_executor_runs_across_the_framed_process_boundary() {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_spool-fake-executor"));
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
            },
        })
        .await
        .expect("execute");
    assert!(matches!(
        response.result,
        Ok(ExecutorResult::Submitted {
            native_job_id: Some(_)
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
