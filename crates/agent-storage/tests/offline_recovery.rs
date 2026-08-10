#![allow(
    clippy::expect_used,
    reason = "fault-test setup and assertions must fail immediately with local context"
)]

use piqae_agent_storage::{AcceptedJob, AgentStore};
use std::collections::HashSet;

fn cloud_job() -> AcceptedJob {
    AcceptedJob {
        job_id: "job_offline_recovery".into(),
        submission_id: "submission_offline_recovery".into(),
        printer_id: "printer_virtual".into(),
        printer_native_id: "virtual-native-queue".into(),
        title: "Deterministic offline recovery".into(),
        content_sha256: "verified-content".into(),
        content_path: "/virtual/content".into(),
        content_kind: "pdf".into(),
        options_json: "{}".into(),
        expires_unix_ms: None,
        accepted_unix_ms: 10,
        cloud_managed: true,
    }
}

fn numbered_cloud_job(number: usize) -> AcceptedJob {
    let mut job = cloud_job();
    job.job_id = format!("job_soak_{number:04}");
    job.submission_id = format!("submission_soak_{number:04}");
    job.content_sha256 = format!("verified-content-{number:04}");
    job
}

#[test]
fn acceptance_intent_queue_and_event_cursor_survive_separate_restart_windows() {
    let directory = tempfile::tempdir().expect("isolated state");
    let database = directory.path().join("agent.sqlite3");

    {
        let mut store = AgentStore::open(&database).expect("initial store");
        let prepared = store
            .prepare_cloud_job(&cloud_job(), "lease-1", "redacted-token", 30_000)
            .expect("durable acceptance intent");
        assert_eq!(prepared.state, "cloud_accept_pending");
        assert!(store.runnable_heads(20).expect("queue").is_empty());
        assert!(
            store
                .pending_cloud_events(0, 10)
                .expect("outbox")
                .is_empty()
        );
    }

    {
        let mut store = AgentStore::open(&database).expect("restart before response");
        let intents = store.pending_cloud_accepts().expect("acceptance replay");
        assert_eq!(intents.len(), 1);
        assert_eq!(intents[0].lease_id, "lease-1");
        assert!(!format!("{:?}", intents[0]).contains("redacted-token"));

        store
            .activate_cloud_job("job_offline_recovery", 40)
            .expect("remote acceptance confirmed");
        store
            .activate_cloud_job("job_offline_recovery", 41)
            .expect("activation replay is idempotent");
    }

    let event_id = {
        let store = AgentStore::open(&database).expect("restart while offline");
        let runnable = store.runnable_heads(50).expect("local queue recovery");
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].job_id, "job_offline_recovery");
        assert!(store.pending_cloud_accepts().expect("no intent").is_empty());
        let events = store.pending_cloud_events(0, 10).expect("durable event");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].state, "queued_local");
        events[0].event_id.clone()
    };

    {
        let mut store = AgentStore::open(&database).expect("reconnect store");
        assert_eq!(
            store
                .acknowledge_cloud_event(&event_id, 60)
                .expect("server cursor acknowledgement"),
            1
        );
    }

    let store = AgentStore::open(&database).expect("post-ack restart");
    assert!(
        store
            .pending_cloud_events(0, 10)
            .expect("outbox after ack")
            .is_empty()
    );
    assert_eq!(store.runnable_heads(70).expect("queue after ack").len(), 1);
    assert!(store.integrity_check().expect("SQLite integrity"));
}

#[test]
fn accelerated_disconnect_retry_soak_has_no_loss_or_duplicate_activation() {
    const JOBS: usize = 250;
    let directory = tempfile::tempdir().expect("isolated state");
    let database = directory.path().join("soak.sqlite3");
    let mut disconnects = 0_usize;
    let mut replayed_accepts = 0_usize;

    for number in 0..JOBS {
        {
            let mut store = AgentStore::open(&database).expect("connect to prepare");
            store
                .prepare_cloud_job(
                    &numbered_cloud_job(number),
                    &format!("lease-{number}"),
                    "redacted-token",
                    60_000,
                )
                .expect("persist acceptance intent before disconnect");
        }
        disconnects += 1;

        {
            let mut store = AgentStore::open(&database).expect("reconnect to accept");
            let timestamp = i64::try_from(number).expect("bounded fixture number");
            assert!(
                store
                    .pending_cloud_accepts()
                    .expect("replayed intents")
                    .iter()
                    .any(|intent| intent.job_id == format!("job_soak_{number:04}"))
            );
            store
                .activate_cloud_job(&format!("job_soak_{number:04}"), 100 + timestamp)
                .expect("first acceptance response");
            store
                .activate_cloud_job(&format!("job_soak_{number:04}"), 101 + timestamp)
                .expect("ambiguous acceptance response replay");
            replayed_accepts += 1;
        }
        disconnects += 1;
    }

    let mut store = AgentStore::open(&database).expect("final reconnect");
    // One printer intentionally exposes only its ordered head as runnable.
    let runnable = store.runnable_heads(1_000_000).expect("durable queue head");
    let events = store
        .pending_cloud_events(0, JOBS + 1)
        .expect("durable event outbox");
    let event_jobs: HashSet<_> = events.iter().map(|event| event.job_id.as_str()).collect();

    assert_eq!(
        runnable.len(),
        1,
        "per-printer ordering must retain one head"
    );
    for number in 0..JOBS {
        let job_id = format!("job_soak_{number:04}");
        let job = store.get_job(&job_id).expect("query durable job");
        assert!(job.is_some(), "lost durable job {job_id}");
        assert_eq!(
            job.as_ref().map(|job| job.state.as_str()),
            Some("queued_local")
        );
    }
    assert_eq!(events.len(), JOBS, "lost or duplicate activation event");
    assert_eq!(
        event_jobs.len(),
        JOBS,
        "duplicate activation event identity"
    );
    assert!(
        store
            .pending_cloud_accepts()
            .expect("drained intents")
            .is_empty()
    );

    for event in &events {
        assert_eq!(
            store
                .acknowledge_cloud_event(&event.event_id, 10_000)
                .expect("first cursor acknowledgement"),
            1
        );
        assert_eq!(
            store
                .acknowledge_cloud_event(&event.event_id, 10_001)
                .expect("duplicate cursor acknowledgement"),
            0
        );
    }
    assert!(
        store
            .pending_cloud_events(0, 1)
            .expect("empty outbox")
            .is_empty()
    );
    assert!(store.integrity_check().expect("SQLite integrity"));
    assert_eq!(disconnects, JOBS * 2);
    assert_eq!(replayed_accepts, JOBS);
}
