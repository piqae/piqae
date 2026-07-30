#![allow(clippy::expect_used)]

#[cfg(unix)]
#[tokio::test]
async fn cups_discovery_runs_across_the_supervised_boundary() {
    use piqae_executor_supervisor::ExecutorSupervisor;
    use piqae_protocol::executor::{ExecutorOperation, ExecutorRequest, ExecutorResult};
    use std::{path::PathBuf, time::Duration};
    use uuid::Uuid;

    let supervisor = ExecutorSupervisor::new(
        PathBuf::from(env!("CARGO_BIN_EXE_piqae-executor-cups")),
        Duration::from_secs(10),
    );
    let response = supervisor
        .execute(&ExecutorRequest {
            request_id: Uuid::nil(),
            deadline_unix_ms: i64::MAX,
            operation: ExecutorOperation::DiscoverPrinters,
        })
        .await
        .expect("CUPS discovery executor");
    assert!(matches!(
        response.result,
        Ok(ExecutorResult::Printers { .. })
    ));
}
