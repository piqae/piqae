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
    #[error("executor framing failed: {0}")]
    Frame(#[from] FrameError),
    #[error("executor timed out")]
    TimedOut,
    #[error("executor exited unsuccessfully: {0}")]
    Exit(std::process::ExitStatus),
    #[error("executor response request ID did not match")]
    RequestMismatch,
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
        write_frame_async(&mut stdin, request).await?;
        drop(stdin);

        let response = if let Ok(response) = timeout(
            request_timeout(request.deadline_unix_ms, self.hard_timeout),
            read_frame_async::<ExecutorResponse>(&mut stdout),
        )
        .await
        {
            response?
        } else {
            terminate(&mut child).await;
            return Err(SupervisorError::TimedOut);
        };
        let status = child.wait().await.map_err(SupervisorError::Spawn)?;
        if !status.success() {
            return Err(SupervisorError::Exit(status));
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
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(SupervisorError::Spawn)
    }
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
                retryable: !matches!(error, SupervisorError::TimedOut),
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
            },
        };
        match self.supervisor.execute(&request).await {
            Ok(ExecutorResponse {
                result:
                    Ok(ExecutorResult::Submitted {
                        native_job_id: Some(native_job_id),
                    }),
                ..
            }) => Ok(NativeAcceptance { native_job_id }),
            Ok(ExecutorResponse {
                result:
                    Ok(ExecutorResult::Submitted {
                        native_job_id: None,
                    }),
                ..
            }) => Err(ExecutorFailure {
                code: "native_job_id_missing".into(),
                message: "spooler accepted the job without an observable ID".into(),
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
                retryable: !matches!(error, SupervisorError::TimedOut),
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
        SupervisorError::TimedOut => "executor_timed_out",
        SupervisorError::Exit(_) => "executor_crashed",
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
    use super::*;

    #[test]
    fn timeout_is_distinct_from_a_process_crash() {
        assert_eq!(
            classify_supervisor_error(&SupervisorError::TimedOut),
            "executor_timed_out"
        );
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            let exit = std::process::ExitStatus::from_raw(256);
            assert_eq!(
                classify_supervisor_error(&SupervisorError::Exit(exit)),
                "executor_crashed"
            );
        }
    }
}
