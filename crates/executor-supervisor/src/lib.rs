//! Killable, one-request-per-process native executor supervisor.

use async_trait::async_trait;
use piqae_agent_core::{
    Executor, ExecutorFailure, LocalSubmission, NativeAcceptance, NativeJobReference,
};
use piqae_domain::{ContentKind, JobId};
use piqae_executor_protocol::{FrameError, read_frame_async, write_frame_async};
use piqae_protocol::executor::{
    ExecutorOperation, ExecutorRequest, ExecutorResponse, ExecutorResult, NativeJobObservation,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use tokio::{
    process::{Child, Command},
    time::{Duration, timeout},
};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum SupervisorError {
    #[error("failed to start executor: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("executor pipe was unavailable")]
    MissingPipe,
    #[error("executor framing failed: {source} ({evidence})")]
    Frame {
        #[source]
        source: FrameError,
        evidence: CrashEvidence,
    },
    #[error("executor timed out ({0})")]
    TimedOut(CrashEvidence),
    #[error("executor exited unsuccessfully: {0} ({1})")]
    Exit(std::process::ExitStatus, CrashEvidence),
    #[error("executor response request ID did not match")]
    RequestMismatch,
}

/// Privacy-minimised evidence for correlating native crashes without retaining
/// raw stderr, which may contain document paths, queue names, or driver data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashEvidence {
    observed_bytes: u64,
    inspected_bytes: usize,
    classification: &'static str,
    drain_complete: bool,
}

impl std::fmt::Display for CrashEvidence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "stderr_class={}, stderr_bytes={}, inspected_bytes={}, stderr_drain_complete={}",
            self.classification, self.observed_bytes, self.inspected_bytes, self.drain_complete
        )
    }
}

#[derive(Debug, Clone)]
pub struct ExecutorSupervisor {
    program: PathBuf,
    hard_timeout: Duration,
}

impl ExecutorSupervisor {
    #[must_use]
    pub const fn new(program: PathBuf, hard_timeout: Duration) -> Self {
        Self {
            program,
            hard_timeout,
        }
    }

    /// Executes one operation in a disposable child process.
    ///
    /// # Errors
    ///
    /// Returns an error when the process cannot start, communicate, complete
    /// within its deadline, or return a matching successful response frame.
    pub async fn execute(
        &self,
        request: &ExecutorRequest,
    ) -> Result<ExecutorResponse, SupervisorError> {
        let mut child = self.spawn()?;
        let mut stdin = child.stdin.take().ok_or(SupervisorError::MissingPipe)?;
        let mut stdout = child.stdout.take().ok_or(SupervisorError::MissingPipe)?;
        let stderr = child.stderr.take().ok_or(SupervisorError::MissingPipe)?;
        let stderr_state = Arc::new(Mutex::new(CrashEvidence::empty()));
        let stderr_task = tokio::spawn(capture_stderr_evidence(stderr, Arc::clone(&stderr_state)));
        let deadline = tokio::time::Instant::now()
            + request_timeout(request.deadline_unix_ms, self.hard_timeout);
        if let Err(source) = write_frame_async(&mut stdin, request).await {
            let status = timeout(
                deadline.saturating_duration_since(tokio::time::Instant::now()),
                child.wait(),
            )
            .await;
            return match status {
                Ok(Ok(status)) if !status.success() => Err(SupervisorError::Exit(
                    status,
                    stderr_evidence(stderr_task, &stderr_state).await,
                )),
                Ok(Ok(_)) => Err(SupervisorError::Frame {
                    source,
                    evidence: stderr_evidence(stderr_task, &stderr_state).await,
                }),
                Ok(Err(error)) => Err(SupervisorError::Spawn(error)),
                Err(_) => {
                    terminate(&mut child).await;
                    Err(SupervisorError::TimedOut(
                        stderr_evidence(stderr_task, &stderr_state).await,
                    ))
                }
            };
        }
        drop(stdin);

        let response = match timeout(
            deadline.saturating_duration_since(tokio::time::Instant::now()),
            read_frame_async::<ExecutorResponse>(&mut stdout),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(source)) => {
                let status = timeout(
                    deadline.saturating_duration_since(tokio::time::Instant::now()),
                    child.wait(),
                )
                .await;
                match status {
                    Ok(Ok(status)) if !status.success() => {
                        return Err(SupervisorError::Exit(
                            status,
                            stderr_evidence(stderr_task, &stderr_state).await,
                        ));
                    }
                    Ok(Ok(_)) => {
                        return Err(SupervisorError::Frame {
                            source,
                            evidence: stderr_evidence(stderr_task, &stderr_state).await,
                        });
                    }
                    Ok(Err(error)) => return Err(SupervisorError::Spawn(error)),
                    Err(_) => {
                        terminate(&mut child).await;
                        return Err(SupervisorError::TimedOut(
                            stderr_evidence(stderr_task, &stderr_state).await,
                        ));
                    }
                }
            }
            Err(_) => {
                terminate(&mut child).await;
                return Err(SupervisorError::TimedOut(
                    stderr_evidence(stderr_task, &stderr_state).await,
                ));
            }
        };
        let status = if let Ok(result) = timeout(
            deadline.saturating_duration_since(tokio::time::Instant::now()),
            child.wait(),
        )
        .await
        {
            result.map_err(SupervisorError::Spawn)?
        } else {
            terminate(&mut child).await;
            return Err(SupervisorError::TimedOut(
                stderr_evidence(stderr_task, &stderr_state).await,
            ));
        };
        let evidence = stderr_evidence(stderr_task, &stderr_state).await;
        if !status.success() {
            return Err(SupervisorError::Exit(status, evidence));
        }
        if response.request_id != request.request_id {
            return Err(SupervisorError::RequestMismatch);
        }
        Ok(response)
    }

    fn spawn(&self) -> Result<Child, SupervisorError> {
        Command::new(&self.program)
            .kill_on_drop(true)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(SupervisorError::Spawn)
    }
}

impl CrashEvidence {
    const fn empty() -> Self {
        Self {
            observed_bytes: 0,
            inspected_bytes: 0,
            classification: "none",
            drain_complete: false,
        }
    }
}

const MAX_STDERR_INSPECTION_BYTES: usize = 64 * 1024;

fn classify_stderr(bytes: &[u8]) -> &'static str {
    let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    if text.contains("access is denied") || text.contains("permission denied") {
        "access_denied"
    } else if text.contains("not found")
        || text.contains("no such file")
        || text.contains("missing")
    {
        "missing_dependency"
    } else if text.contains("panic") || text.contains("fatal") {
        "native_crash"
    } else if text.contains("driver") {
        "driver_failure"
    } else if bytes.is_empty() {
        "none"
    } else {
        "unclassified"
    }
}

async fn capture_stderr_evidence(
    mut stderr: impl tokio::io::AsyncRead + Unpin,
    state: Arc<Mutex<CrashEvidence>>,
) {
    use tokio::io::AsyncReadExt as _;

    let mut observed_bytes = 0_u64;
    let mut inspected = Vec::with_capacity(MAX_STDERR_INSPECTION_BYTES);
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let Ok(count) = stderr.read(&mut buffer).await else {
            break;
        };
        if count == 0 {
            break;
        }
        observed_bytes = observed_bytes.saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
        let remaining = MAX_STDERR_INSPECTION_BYTES.saturating_sub(inspected.len());
        inspected.extend_from_slice(&buffer[..count.min(remaining)]);
        if let Ok(mut evidence) = state.lock() {
            evidence.observed_bytes = observed_bytes;
            evidence.inspected_bytes = inspected.len();
            evidence.classification = classify_stderr(&inspected);
        }
    }
    if let Ok(mut evidence) = state.lock() {
        evidence.drain_complete = true;
    }
}

async fn stderr_evidence(
    mut task: tokio::task::JoinHandle<()>,
    state: &Arc<Mutex<CrashEvidence>>,
) -> CrashEvidence {
    if timeout(Duration::from_secs(2), &mut task).await.is_err() {
        task.abort();
        let _ = task.await;
    }
    state
        .lock()
        .map_or_else(|_| CrashEvidence::empty(), |evidence| evidence.clone())
}

fn request_timeout(deadline_unix_ms: i64, hard_timeout: Duration) -> Duration {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let now_ms = i64::try_from(now.as_millis()).unwrap_or(i64::MAX);
    let remaining_ms = deadline_unix_ms.saturating_sub(now_ms).max(0);
    hard_timeout.min(Duration::from_millis(
        u64::try_from(remaining_ms).unwrap_or(u64::MAX),
    ))
}

async fn terminate(child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[derive(Debug, Clone)]
pub struct SupervisedExecutor {
    supervisor: ExecutorSupervisor,
}

impl SupervisedExecutor {
    #[must_use]
    pub const fn new(supervisor: ExecutorSupervisor) -> Self {
        Self { supervisor }
    }

    /// Executes a non-submission native operation.
    ///
    /// # Errors
    ///
    /// Returns an executor failure when the child cannot complete the
    /// operation or reports a native error.
    pub async fn execute_operation(
        &self,
        operation: ExecutorOperation,
        deadline_unix_ms: i64,
    ) -> Result<ExecutorResult, ExecutorFailure> {
        let request = ExecutorRequest {
            request_id: Uuid::new_v4(),
            deadline_unix_ms,
            operation,
        };
        match self.supervisor.execute(&request).await {
            Ok(ExecutorResponse {
                result: Ok(result), ..
            }) => Ok(result),
            Ok(ExecutorResponse {
                result: Err(error), ..
            }) => Err(ExecutorFailure {
                code: error.code,
                message: error.message,
                retryable: error.retryable,
                handoff_may_have_succeeded: error.handoff_may_have_succeeded,
                native_code: None,
            }),
            Err(error) => Err(ExecutorFailure {
                code: classify_supervisor_error(&error).into(),
                message: error.to_string(),
                retryable: !matches!(error, SupervisorError::TimedOut(_)),
                handoff_may_have_succeeded: false,
                native_code: None,
            }),
        }
    }
}

#[async_trait]
impl Executor for SupervisedExecutor {
    async fn submit(
        &mut self,
        submission: LocalSubmission,
    ) -> Result<NativeAcceptance, ExecutorFailure> {
        let expected_fence = submission.route_fence.clone();
        let job_id = submission
            .job_id
            .parse::<JobId>()
            .map_err(|_| invalid_local_job_id())?;
        let content_kind = if submission.content_kind == "pdf" {
            ContentKind::Pdf
        } else if submission.content_kind == "raw" {
            ContentKind::Raw
        } else {
            return Err(ExecutorFailure {
                code: "unsupported_content_kind".into(),
                message: "executor supports only PDF and RAW content".into(),
                retryable: false,
                handoff_may_have_succeeded: false,
                native_code: None,
            });
        };
        let request = ExecutorRequest {
            request_id: Uuid::new_v4(),
            deadline_unix_ms: submission.deadline_unix_ms,
            operation: ExecutorOperation::Submit {
                job_id,
                native_printer_id: submission.printer_native_id,
                title: submission.title,
                content_kind,
                content_path: submission.content_path.to_string_lossy().into_owned(),
                options: submission.options,
                native_profile: submission.native_profile,
                route_fence: submission.route_fence,
            },
        };
        match self.supervisor.execute(&request).await {
            Ok(ExecutorResponse {
                result:
                    Ok(ExecutorResult::Submitted {
                        native_job_id: Some(native_job_id),
                        route_fence,
                    }),
                ..
            }) if route_fence == expected_fence => Ok(NativeAcceptance { native_job_id }),
            Ok(ExecutorResponse {
                result:
                    Ok(ExecutorResult::Submitted {
                        native_job_id: None,
                        route_fence,
                    }),
                ..
            }) if route_fence == expected_fence => Err(ExecutorFailure {
                code: "native_job_id_missing".into(),
                message: "spooler accepted the job without an observable ID".into(),
                retryable: false,
                handoff_may_have_succeeded: true,
                native_code: None,
            }),
            Ok(ExecutorResponse {
                result: Ok(ExecutorResult::Submitted { .. }),
                ..
            }) => Err(ExecutorFailure {
                code: "stale_route_fence".into(),
                message: "native executor did not echo the active route fence".into(),
                retryable: false,
                handoff_may_have_succeeded: true,
                native_code: None,
            }),
            Ok(ExecutorResponse {
                result: Err(error), ..
            }) => Err(ExecutorFailure {
                code: error.code,
                message: error.message,
                retryable: error.retryable,
                handoff_may_have_succeeded: error.handoff_may_have_succeeded,
                native_code: None,
            }),
            Ok(_) => Err(ExecutorFailure {
                code: "unexpected_executor_response".into(),
                message: "executor returned the wrong operation result".into(),
                retryable: false,
                handoff_may_have_succeeded: false,
                native_code: None,
            }),
            Err(error) => Err(ExecutorFailure {
                code: classify_supervisor_error(&error).into(),
                message: error.to_string(),
                retryable: !matches!(error, SupervisorError::TimedOut(_)),
                // The request was fully written before we awaited a response.
                // A timeout or process failure can therefore be ambiguous.
                handoff_may_have_succeeded: true,
                native_code: None,
            }),
        }
    }

    async fn observe(
        &mut self,
        reference: NativeJobReference,
    ) -> Result<NativeJobObservation, ExecutorFailure> {
        match self
            .execute_operation(
                ExecutorOperation::Observe {
                    native_printer_id: reference.printer_native_id,
                    native_job_id: reference.native_job_id,
                },
                reference.deadline_unix_ms,
            )
            .await?
        {
            ExecutorResult::Observation { observation } => Ok(observation),
            _ => Err(unexpected_response("observation")),
        }
    }

    async fn cancel(&mut self, reference: NativeJobReference) -> Result<(), ExecutorFailure> {
        match self
            .execute_operation(
                ExecutorOperation::Cancel {
                    native_printer_id: reference.printer_native_id,
                    native_job_id: reference.native_job_id,
                },
                reference.deadline_unix_ms,
            )
            .await?
        {
            ExecutorResult::Cancelled => Ok(()),
            _ => Err(unexpected_response("cancellation")),
        }
    }
}

const fn classify_supervisor_error(error: &SupervisorError) -> &'static str {
    match error {
        SupervisorError::TimedOut(_) => "executor_timed_out",
        SupervisorError::Exit(_, _) => "executor_crashed",
        _ => "executor_failed",
    }
}

fn unexpected_response(operation: &str) -> ExecutorFailure {
    ExecutorFailure {
        code: "unexpected_executor_response".into(),
        message: format!("executor returned the wrong {operation} result"),
        retryable: false,
        handoff_may_have_succeeded: false,
        native_code: None,
    }
}

fn invalid_local_job_id() -> ExecutorFailure {
    ExecutorFailure {
        code: "invalid_local_job_id".into(),
        message: "local job ID is not a canonical Piqae job ID".into(),
        retryable: false,
        handoff_may_have_succeeded: false,
        native_code: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CrashEvidence, SupervisorError, capture_stderr_evidence, classify_supervisor_error,
        stderr_evidence,
    };
    use std::sync::{Arc, Mutex};
    use tokio::io::AsyncWriteExt as _;

    #[test]
    fn timeout_is_distinct_from_a_process_crash() {
        assert_eq!(
            classify_supervisor_error(&SupervisorError::TimedOut(CrashEvidence::empty())),
            "executor_timed_out"
        );
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            let exit = std::process::ExitStatus::from_raw(256);
            assert_eq!(
                classify_supervisor_error(&SupervisorError::Exit(exit, CrashEvidence::empty())),
                "executor_crashed"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn child_exit_before_response_is_classified_as_a_crash() {
        let supervisor = super::ExecutorSupervisor::new(
            std::path::PathBuf::from("/usr/bin/false"),
            std::time::Duration::from_secs(2),
        );
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let request = piqae_protocol::executor::ExecutorRequest {
            request_id: uuid::Uuid::new_v4(),
            deadline_unix_ms: i64::try_from(now.as_millis())
                .unwrap_or(i64::MAX)
                .saturating_add(2_000),
            operation: piqae_protocol::executor::ExecutorOperation::DiscoverPrinters,
        };

        let Err(error) = supervisor.execute(&request).await else {
            panic!("child exit unexpectedly succeeded");
        };
        assert!(matches!(error, SupervisorError::Exit(_, _)));
        assert_eq!(classify_supervisor_error(&error), "executor_crashed");
    }

    #[tokio::test]
    async fn stderr_evidence_is_bounded_and_contains_no_raw_output() {
        let (reader, mut writer) = tokio::io::duplex(1024);
        let sensitive = vec![b'x'; 64 * 1024 + 17];
        let state = Arc::new(Mutex::new(CrashEvidence::empty()));
        let write = tokio::spawn(async move {
            if let Err(error) = writer.write_all(&sensitive).await {
                panic!("write fixture: {error}");
            }
        });
        capture_stderr_evidence(reader, Arc::clone(&state)).await;
        if let Err(error) = write.await {
            panic!("writer task: {error}");
        }

        let evidence = state
            .lock()
            .map_or_else(|_| CrashEvidence::empty(), |value| value.clone());
        assert_eq!(
            evidence.observed_bytes,
            u64::try_from(64 * 1024 + 17).unwrap_or_default()
        );
        assert_eq!(evidence.inspected_bytes, 64 * 1024);
        assert_eq!(evidence.classification, "unclassified");
        assert!(evidence.drain_complete);
        assert!(!evidence.to_string().contains("xxxx"));
    }

    #[tokio::test]
    async fn inherited_stderr_handle_is_cancelled_after_bounded_wait() {
        let (reader, mut writer) = tokio::io::duplex(128);
        writer
            .write_all(b"permission denied: private path")
            .await
            .unwrap_or_default();
        let state = Arc::new(Mutex::new(CrashEvidence::empty()));
        let task = tokio::spawn(capture_stderr_evidence(reader, Arc::clone(&state)));
        let evidence = stderr_evidence(task, &state).await;
        assert_eq!(evidence.classification, "access_denied");
        assert!(!evidence.drain_complete);
        assert!(!evidence.to_string().contains("private path"));
        drop(writer);
    }
}
