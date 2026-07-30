#![allow(
    clippy::expect_used,
    reason = "fault-test setup and assertions must fail immediately with local context"
)]

use piqae_agent_storage::{AcceptedJob, AgentStore};

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
