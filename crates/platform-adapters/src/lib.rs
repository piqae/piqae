//! Platform printing boundary.
//!
//! Native integrations implement this trait in separately killable executor
//! binaries. Unsupported builds fail closed and can never report a successful
//! submission.

use async_trait::async_trait;
use piqae_protocol::executor::{
    DiscoveredPrinter, ExecutorError, ExecutorOperation, ExecutorResult, NativeJobObservation,
    NativeJobState,
};

#[async_trait]
pub trait PlatformAdapter: Send {
    fn name(&self) -> &'static str;

    async fn execute(
        &mut self,
        operation: ExecutorOperation,
    ) -> Result<ExecutorResult, ExecutorError>;
}

#[derive(Debug, Clone)]
pub struct UnsupportedAdapter {
    platform: &'static str,
}

impl UnsupportedAdapter {
    #[must_use]
    pub const fn new(platform: &'static str) -> Self {
        Self { platform }
    }
}

#[async_trait]
impl PlatformAdapter for UnsupportedAdapter {
    fn name(&self) -> &'static str {
        self.platform
    }

    async fn execute(
        &mut self,
        _operation: ExecutorOperation,
    ) -> Result<ExecutorResult, ExecutorError> {
        Err(ExecutorError {
            code: "platform_adapter_unavailable".into(),
            message: format!(
                "the {} native print adapter is not included in this build",
                self.platform
            ),
            retryable: false,
            handoff_may_have_succeeded: false,
        })
    }
}

/// Deterministic adapter for contract and resilience testing. It never calls a
/// host spooler.
#[derive(Debug, Default)]
pub struct FakePlatformAdapter {
    pub printers: Vec<DiscoveredPrinter>,
    pub submitted_jobs: usize,
}

#[async_trait]
impl PlatformAdapter for FakePlatformAdapter {
    fn name(&self) -> &'static str {
        "fake"
    }

    async fn execute(
        &mut self,
        operation: ExecutorOperation,
    ) -> Result<ExecutorResult, ExecutorError> {
        match operation {
            ExecutorOperation::DiscoverPrinters => Ok(ExecutorResult::Printers {
                printers: self.printers.clone(),
            }),
            ExecutorOperation::GetPrinterState { native_printer_id } => self
                .printers
                .iter()
                .find(|printer| printer.native_id == native_printer_id)
                .map(|printer| ExecutorResult::State {
                    state: printer.state,
                })
                .ok_or_else(|| not_found(&native_printer_id)),
            ExecutorOperation::GetPrinterCapabilities { native_printer_id } => self
                .printers
                .iter()
                .find(|printer| printer.native_id == native_printer_id)
                .map(|printer| ExecutorResult::Capabilities {
                    capabilities: printer.capabilities.clone(),
                    native_options: printer.native_options.clone(),
                })
                .ok_or_else(|| not_found(&native_printer_id)),
            ExecutorOperation::ListJobs { native_printer_id } => self
                .printers
                .iter()
                .any(|printer| printer.native_id == native_printer_id)
                .then(|| ExecutorResult::Jobs { jobs: Vec::new() })
                .ok_or_else(|| not_found(&native_printer_id)),
            ExecutorOperation::Submit {
                native_printer_id,
                route_fence,
                ..
            } => {
                if !self
                    .printers
                    .iter()
                    .any(|printer| printer.native_id == native_printer_id)
                {
                    return Err(not_found(&native_printer_id));
                }
                self.submitted_jobs += 1;
                Ok(ExecutorResult::Submitted {
                    native_job_id: Some(format!("fake-{}", self.submitted_jobs)),
                    route_fence,
                })
            }
            ExecutorOperation::Cancel {
                native_printer_id, ..
            } => {
                if !self
                    .printers
                    .iter()
                    .any(|printer| printer.native_id == native_printer_id)
                {
                    return Err(not_found(&native_printer_id));
                }
                Ok(ExecutorResult::Cancelled)
            }
            ExecutorOperation::Observe {
                native_printer_id, ..
            } => {
                if !self
                    .printers
                    .iter()
                    .any(|printer| printer.native_id == native_printer_id)
                {
                    return Err(not_found(&native_printer_id));
                }
                Ok(ExecutorResult::Observation {
                    observation: NativeJobObservation {
                        state: NativeJobState::Completed,
                        native_code: Some("fake".into()),
                        message: Some("Fake adapter reports completion".into()),
                    },
                })
            }
        }
    }
}

fn not_found(native_id: &str) -> ExecutorError {
    ExecutorError {
        code: "printer_not_found".into(),
        message: format!("native printer {native_id} was not found"),
        retryable: false,
        handoff_may_have_succeeded: false,
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use piqae_domain::{PrinterCapabilities, PrinterState};

    #[tokio::test]
    async fn unsupported_adapter_fails_closed() {
        let mut adapter = UnsupportedAdapter::new("windows");
        let error = adapter
            .execute(ExecutorOperation::DiscoverPrinters)
            .await
            .expect_err("must fail");
        assert!(!error.retryable);
        assert!(!error.handoff_may_have_succeeded);
    }

    #[tokio::test]
    async fn fake_adapter_returns_stable_native_ids() {
        let mut adapter = FakePlatformAdapter {
            printers: vec![DiscoveredPrinter {
                native_id: "printer".into(),
                name: "Test".into(),
                is_default: true,
                state: PrinterState::Online,
                capabilities: PrinterCapabilities::default(),
                native_options: std::collections::BTreeMap::new(),
                driver_fingerprint: None,
                identity_evidence: Vec::new(),
            }],
            submitted_jobs: 0,
        };
        let result = adapter
            .execute(ExecutorOperation::DiscoverPrinters)
            .await
            .expect("discover");
        assert!(matches!(result, ExecutorResult::Printers { .. }));
    }
}
