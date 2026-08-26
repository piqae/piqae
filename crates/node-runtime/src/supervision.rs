//! Pure durable-connector supervision planner.
//!
//! Process hosts execute the returned stop/start actions, while this module
//! remains the single place that decides whether a connector must continue,
//! restart after failure or rotate after its durable authorization changes.

use crate::connector_registry::{ConnectorRecord, ConnectorRegistry};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct WorkerObservation<'a> {
    pub record: &'a ConnectorRecord,
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorReconciliation {
    /// Worker IDs to stop before any replacements are started.
    pub stop: Vec<String>,
    /// Desired durable records to start after stops have completed.
    pub start: Vec<ConnectorRecord>,
}

#[must_use]
pub fn plan_connector_reconciliation(
    registry: &ConnectorRegistry,
    workers: &BTreeMap<String, WorkerObservation<'_>>,
) -> ConnectorReconciliation {
    let desired = registry
        .enabled()
        .map(|record| (record.connector_id.clone(), record))
        .collect::<BTreeMap<_, _>>();
    let mut stop = BTreeSet::new();
    let mut start = Vec::new();

    for (connector_id, worker) in workers {
        let matches = desired
            .get(connector_id)
            .is_some_and(|record| worker.running && *record == worker.record);
        if !matches {
            stop.insert(connector_id.clone());
        }
    }
    for (connector_id, record) in desired {
        let matches = workers
            .get(&connector_id)
            .is_some_and(|worker| worker.running && worker.record == record);
        if !matches {
            start.push(record.clone());
        }
    }
    ConnectorReconciliation {
        stop: stop.into_iter().collect(),
        start,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use piqae_protocol::agent::PrinterGrant;
    use std::path::PathBuf;
    use url::Url;

    fn record(id: &str) -> ConnectorRecord {
        ConnectorRecord {
            connector_id: id.into(),
            agent_id: "agt_test".into(),
            control_plane_url: Url::parse("https://api.piqae.test").unwrap(),
            display_name: None,
            workspace_name: None,
            authorization_type: None,
            workspace_id: None,
            environment_id: None,
            requesting_service_account_id: None,
            manage_url: None,
            device_key_file: PathBuf::from(format!("connectors/{id}/device.key")),
            enabled: true,
            printer_grant: PrinterGrant::AllLocalPrinters,
            allowed_printer_ids: Vec::new(),
        }
    }

    #[test]
    fn changed_failed_and_removed_workers_are_stopped_before_restart() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = ConnectorRegistry::load(directory.path()).unwrap();
        registry.add(record("ncon_current")).unwrap();
        registry.add(record("ncon_failed")).unwrap();
        let stale = record("ncon_removed");
        let current = record("ncon_current");
        let failed = record("ncon_failed");
        let workers = BTreeMap::from([
            (
                "ncon_current".into(),
                WorkerObservation {
                    record: &current,
                    running: true,
                },
            ),
            (
                "ncon_failed".into(),
                WorkerObservation {
                    record: &failed,
                    running: false,
                },
            ),
            (
                "ncon_removed".into(),
                WorkerObservation {
                    record: &stale,
                    running: true,
                },
            ),
        ]);
        let plan = plan_connector_reconciliation(&registry, &workers);
        assert_eq!(plan.stop, vec!["ncon_failed", "ncon_removed"]);
        assert_eq!(
            plan.start
                .iter()
                .map(|record| record.connector_id.as_str())
                .collect::<Vec<_>>(),
            vec!["ncon_failed"]
        );
    }
}
