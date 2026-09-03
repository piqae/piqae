#![allow(clippy::expect_used)]

use piqae_domain::{AgentId, EventId, JobFailureReason, JobId, JobState};
use piqae_storage_postgres::{DeliveryAttemptProof, PostgresStore, StorageError};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{borrow::Cow, env, path::Path, process::Command};
use uuid::Uuid;

#[derive(Clone, Copy)]
struct AcceptanceMigrationFixture {
    name: &'static str,
    workspace_id: &'static str,
    environment_id: &'static str,
    agent_id: &'static str,
    printer_id: &'static str,
    job_id: &'static str,
    lease_id: &'static str,
    job_state: &'static str,
    attempt_state: &'static str,
    revoked: bool,
}

#[tokio::test]
async fn released_v0_1_21_history_is_refused_before_sqlx_checksum_validation() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let listing = Command::new("git")
        .args([
            "ls-tree",
            "-r",
            "--name-only",
            "v0.1.21",
            "--",
            "migrations/postgres",
        ])
        .current_dir(&repository)
        .output()
        .expect("read released v0.1.21 migration inventory");
    assert!(listing.status.success(), "v0.1.21 release tag is required");

    let fixture_root = repository
        .join(".piqae-test-fixtures/postgres-migration")
        .join(ulid::Ulid::new().to_string().to_ascii_lowercase());
    let migration_root = fixture_root.join("migrations");
    std::fs::create_dir_all(&migration_root).expect("create isolated migration fixture");
    for source in String::from_utf8(listing.stdout)
        .expect("migration inventory is UTF-8")
        .lines()
    {
        let filename = Path::new(source)
            .file_name()
            .and_then(|value| value.to_str())
            .expect("released migration has a safe filename");
        let contents = Command::new("git")
            .args(["show", &format!("v0.1.21:{source}")])
            .current_dir(&repository)
            .output()
            .expect("read released migration");
        assert!(contents.status.success(), "released migration is readable");
        std::fs::write(migration_root.join(filename), contents.stdout)
            .expect("write isolated released migration fixture");
    }
    let released = Migrator::new(migration_root)
        .await
        .expect("load released migration fixture");

    let schema = format!("piqae_v021_refusal_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create exact disposable schema");
    let pool = schema_pool(&database_url, &schema).await;
    released
        .run(&pool)
        .await
        .expect("apply exact released v0.1.21 history");

    let error = PostgresStore::from_pool(pool.clone())
        .migrate()
        .await
        .expect_err("v0.1.21 must be refused by the v0.1.22 preflight");
    assert!(matches!(error, StorageError::UnsupportedDatabaseBaseline));

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
    admin.close().await;
    std::fs::remove_dir_all(&fixture_root).expect("remove exact migration fixture");
}

#[tokio::test]
async fn automatic_wake_outbox_upgrades_41_and_is_tenant_isolated() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let schema = format!("piqae_wake_outbox_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create exact disposable schema");
    let pool = schema_pool(&database_url, &schema).await;
    let all = sqlx::migrate!("../../migrations/postgres");
    let previous = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 42)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    previous.run(&pool).await.expect("apply version 41 schema");
    for suffix in ["a", "b"] {
        sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
            .bind(format!("wsp_wake_{suffix}"))
            .bind(format!("Wake {suffix}"))
            .bind(format!("wake-{suffix}"))
            .execute(&pool)
            .await
            .expect("insert workspace fixture");
        sqlx::query(
            "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'test','Test')",
        )
        .bind(format!("env_wake_{suffix}"))
        .bind(format!("wsp_wake_{suffix}"))
        .execute(&pool)
        .await
        .expect("insert environment fixture");
        sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Node',$4,'linux','x86_64','test',1)")
            .bind(format!("agt_wake_{suffix}"))
            .bind(format!("wsp_wake_{suffix}"))
            .bind(format!("env_wake_{suffix}"))
            .bind(format!("install-wake-{suffix}"))
            .execute(&pool).await.expect("insert agent fixture");
        sqlx::query("INSERT INTO printers (id,workspace_id,environment_id,agent_id,native_id,name) VALUES ($1,$2,$3,$4,$5,'Printer')")
            .bind(format!("prt_wake_{suffix}"))
            .bind(format!("wsp_wake_{suffix}"))
            .bind(format!("env_wake_{suffix}"))
            .bind(format!("agt_wake_{suffix}"))
            .bind(format!("native-wake-{suffix}"))
            .execute(&pool).await.expect("insert printer fixture");
        sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,state_sequence,per_printer_sequence,expires_at) VALUES ($1,$2,$3,$4,$5,'{}','waiting_for_agent',2,1,now()+interval '1 hour')")
            .bind(format!("job_wake_{suffix}"))
            .bind(format!("wsp_wake_{suffix}"))
            .bind(format!("env_wake_{suffix}"))
            .bind(format!("prt_wake_{suffix}"))
            .bind(format!("agt_wake_{suffix}"))
            .execute(&pool).await.expect("insert waiting job fixture");
    }
    PostgresStore::from_pool(pool.clone())
        .migrate()
        .await
        .expect("upgrade 43 to automatic wake outbox");
    sqlx::query("INSERT INTO job_wake_reconciliations (workspace_id,environment_id,job_id,state_sequence,candidate_count) VALUES ('wsp_wake_a','env_wake_a','job_wake_a',2,0)")
        .execute(&pool).await.expect("insert zero-candidate tenant reconciliation");
    let cross_tenant = sqlx::query("INSERT INTO job_wake_reconciliations (workspace_id,environment_id,job_id,state_sequence,candidate_count) VALUES ('wsp_wake_b','env_wake_b','job_wake_a',2,1)")
        .execute(&pool).await;
    assert!(
        cross_tenant.is_err(),
        "composite job foreign key must reject a cross-tenant wake marker"
    );
    let other_tenant_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_wake_reconciliations WHERE workspace_id='wsp_wake_b'",
    )
    .fetch_one(&pool)
    .await
    .expect("probe isolated tenant markers");
    assert_eq!(other_tenant_count, 0);
    let latest: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
        .fetch_one(&pool)
        .await
        .expect("read schema version");
    assert_eq!(latest, 47);

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn acceptance_route_reconciliation_upgrades_42_and_fences_tenants() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    let all = sqlx::migrate!("../../migrations/postgres");

    let upgrade_schema =
        format!("piqae_acceptance_upgrade_{}", ulid::Ulid::new()).to_ascii_lowercase();
    sqlx::query(&format!("CREATE SCHEMA {upgrade_schema}"))
        .execute(&admin)
        .await
        .expect("create exact disposable upgrade schema");
    let upgrade_pool = schema_pool(&database_url, &upgrade_schema).await;
    let previous = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 43)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    previous
        .run(&upgrade_pool)
        .await
        .expect("apply version 42 schema");

    let fixtures = [
        AcceptanceMigrationFixture {
            name: "active",
            workspace_id: "wsp_01J00000000000000000000100",
            environment_id: "env_01J00000000000000000000101",
            agent_id: "agt_01J00000000000000000000102",
            printer_id: "ptr_01J00000000000000000000103",
            job_id: "job_01J00000000000000000000104",
            lease_id: "10000000-0000-0000-0000-000000000001",
            job_state: "agent_accepted",
            attempt_state: "queued_local",
            revoked: false,
        },
        AcceptanceMigrationFixture {
            name: "revoked_agent",
            workspace_id: "wsp_01J00000000000000000000200",
            environment_id: "env_01J00000000000000000000201",
            agent_id: "agt_01J00000000000000000000202",
            printer_id: "ptr_01J00000000000000000000203",
            job_id: "job_01J00000000000000000000204",
            lease_id: "20000000-0000-0000-0000-000000000001",
            job_state: "agent_accepted",
            attempt_state: "queued_local",
            revoked: true,
        },
        AcceptanceMigrationFixture {
            name: "revoked_queued",
            workspace_id: "wsp_01J00000000000000000000300",
            environment_id: "env_01J00000000000000000000301",
            agent_id: "agt_01J00000000000000000000302",
            printer_id: "ptr_01J00000000000000000000303",
            job_id: "job_01J00000000000000000000304",
            lease_id: "30000000-0000-0000-0000-000000000001",
            job_state: "queued_local",
            attempt_state: "queued_local",
            revoked: true,
        },
        AcceptanceMigrationFixture {
            name: "legacy",
            workspace_id: "wsp_01J00000000000000000000400",
            environment_id: "env_01J00000000000000000000401",
            agent_id: "agt_01J00000000000000000000402",
            printer_id: "ptr_01J00000000000000000000403",
            job_id: "job_01J00000000000000000000404",
            lease_id: "40000000-0000-0000-0000-000000000001",
            job_state: "agent_accepted",
            attempt_state: "accepted_by_node",
            revoked: false,
        },
        AcceptanceMigrationFixture {
            name: "cross",
            workspace_id: "wsp_01J00000000000000000000500",
            environment_id: "env_01J00000000000000000000501",
            agent_id: "agt_01J00000000000000000000502",
            printer_id: "ptr_01J00000000000000000000503",
            job_id: "job_01J00000000000000000000504",
            lease_id: "50000000-0000-0000-0000-000000000001",
            job_state: "agent_accepted",
            attempt_state: "queued_local",
            revoked: false,
        },
    ];
    for fixture in fixtures {
        sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
            .bind(fixture.workspace_id)
            .bind(format!("Acceptance {}", fixture.name))
            .bind(format!("acceptance-{}", fixture.name))
            .execute(&upgrade_pool)
            .await
            .expect("insert workspace fixture");
        sqlx::query(
            "INSERT INTO environments (id,workspace_id,kind,name)
             VALUES ($1,$2,'test','Test')",
        )
        .bind(fixture.environment_id)
        .bind(fixture.workspace_id)
        .execute(&upgrade_pool)
        .await
        .expect("insert environment fixture");
        sqlx::query(
            "INSERT INTO agents
             (id,workspace_id,environment_id,name,installation_id,os,architecture,version,protocol_version)
             VALUES ($1,$2,$3,'Node',$4,'linux','x86_64','test',1)",
        )
        .bind(fixture.agent_id)
        .bind(fixture.workspace_id)
        .bind(fixture.environment_id)
        .bind(format!("install-accept-{}", fixture.name))
        .execute(&upgrade_pool)
        .await
        .expect("insert agent fixture");
        sqlx::query(
            "INSERT INTO node_installations (id,installation_key,public_key)
             VALUES ($1,$2,$3)",
        )
        .bind(format!("ninst_accept_{}", fixture.name))
        .bind(format!("installation-key-{}", fixture.name))
        .bind(vec![7_u8; 32])
        .execute(&upgrade_pool)
        .await
        .expect("insert node installation fixture");
        sqlx::query(
            "INSERT INTO node_connectors
             (id,installation_id,workspace_id,environment_id,agent_id,revoked_at)
             VALUES ($1,$2,$3,$4,$5,CASE WHEN $6 THEN now() ELSE NULL END)",
        )
        .bind(format!("ncon_accept_{}", fixture.name))
        .bind(format!("ninst_accept_{}", fixture.name))
        .bind(fixture.workspace_id)
        .bind(fixture.environment_id)
        .bind(fixture.agent_id)
        .bind(fixture.revoked)
        .execute(&upgrade_pool)
        .await
        .expect("insert active or revoked connector fixture");
        sqlx::query(
            "INSERT INTO printers
             (id,workspace_id,environment_id,agent_id,native_id,name,state)
             VALUES ($1,$2,$3,$4,$5,'Printer','online')",
        )
        .bind(fixture.printer_id)
        .bind(fixture.workspace_id)
        .bind(fixture.environment_id)
        .bind(fixture.agent_id)
        .bind(format!("native-accept-{}", fixture.name))
        .execute(&upgrade_pool)
        .await
        .expect("insert printer fixture");
        sqlx::query(
            "INSERT INTO physical_destinations
             (workspace_id,environment_id,id,name,state)
             VALUES ($1,$2,$3,'Destination','available')",
        )
        .bind(fixture.workspace_id)
        .bind(fixture.environment_id)
        .bind(format!("pdst_accept_{}", fixture.name))
        .execute(&upgrade_pool)
        .await
        .expect("insert physical destination fixture");
        sqlx::query(
            "INSERT INTO printer_routes
             (workspace_id,environment_id,id,destination_id,printer_id,agent_id,
              native_queue_id,state,role,priority,enabled)
             VALUES ($1,$2,$3,$4,$5,$6,$7,'available','primary',0,true)",
        )
        .bind(fixture.workspace_id)
        .bind(fixture.environment_id)
        .bind(format!("rte_accept_{}", fixture.name))
        .bind(format!("pdst_accept_{}", fixture.name))
        .bind(fixture.printer_id)
        .bind(fixture.agent_id)
        .bind(format!("native-accept-{}", fixture.name))
        .execute(&upgrade_pool)
        .await
        .expect("insert printer route fixture");
        sqlx::query(
            "INSERT INTO jobs
             (id,workspace_id,environment_id,printer_id,agent_id,payload,state,
              state_sequence,per_printer_sequence,expires_at,destination_id,route_id)
             VALUES ($1,$2,$3,$4,$5,'{}',$6,3,1,
                     now()+interval '1 hour',$7,$8)",
        )
        .bind(fixture.job_id)
        .bind(fixture.workspace_id)
        .bind(fixture.environment_id)
        .bind(fixture.printer_id)
        .bind(fixture.agent_id)
        .bind(fixture.job_state)
        .bind(format!("pdst_accept_{}", fixture.name))
        .bind(format!("rte_accept_{}", fixture.name))
        .execute(&upgrade_pool)
        .await
        .expect("insert accepted job fixture");
        let fencing_token = format!("fence-{}", fixture.name);
        let fencing_token_hash = format!("{:x}", Sha256::digest(fencing_token.as_bytes()));
        sqlx::query(
            "INSERT INTO delivery_attempts
             (workspace_id,environment_id,id,job_id,destination_id,route_id,generation,
              fencing_token_hash,state,lease_until,accepted_at)
             VALUES ($1,$2,$3,$4,$5,$6,7,$7,$8,
                     now()+interval '1 hour',now())",
        )
        .bind(fixture.workspace_id)
        .bind(fixture.environment_id)
        .bind(format!("datt_accept_{}", fixture.name))
        .bind(fixture.job_id)
        .bind(format!("pdst_accept_{}", fixture.name))
        .bind(format!("rte_accept_{}", fixture.name))
        .bind(&fencing_token_hash)
        .bind(fixture.attempt_state)
        .execute(&upgrade_pool)
        .await
        .expect("insert delivery attempt fixture");
        sqlx::query(
            "INSERT INTO route_reservations
             (workspace_id,environment_id,id,route_id,destination_id,job_id,attempt_id,
              generation,fencing_token_hash,state,lease_until)
             VALUES ($1,$2,$3,$4,$5,$6,$7,7,$8,'active',now()+interval '1 hour')",
        )
        .bind(fixture.workspace_id)
        .bind(fixture.environment_id)
        .bind(format!("rrsv_accept_{}", fixture.name))
        .bind(format!("rte_accept_{}", fixture.name))
        .bind(format!("pdst_accept_{}", fixture.name))
        .bind(fixture.job_id)
        .bind(format!("datt_accept_{}", fixture.name))
        .bind(fencing_token_hash)
        .execute(&upgrade_pool)
        .await
        .expect("insert route reservation fixture");
        let lease_token = format!("lease-token-{}", fixture.name);
        sqlx::query(
            "INSERT INTO job_acceptances
             (job_id,workspace_id,environment_id,agent_id,lease_id,lease_token_hash,
              content_sha256,local_sequence)
             VALUES ($1,$2,$3,$4,$5,$6,$7,41)",
        )
        .bind(fixture.job_id)
        .bind(fixture.workspace_id)
        .bind(fixture.environment_id)
        .bind(fixture.agent_id)
        .bind(Uuid::parse_str(fixture.lease_id).expect("valid fixture lease UUID"))
        .bind(Sha256::digest(lease_token.as_bytes()).to_vec())
        .bind(format!("content-{}", fixture.name))
        .execute(&upgrade_pool)
        .await
        .expect("insert legacy acceptance fixture");
    }
    sqlx::query(
        "UPDATE job_acceptances
         SET workspace_id=$1,environment_id=$2
         WHERE job_id=$3",
    )
    .bind(fixtures[0].workspace_id)
    .bind(fixtures[0].environment_id)
    .bind(fixtures[4].job_id)
    .execute(&upgrade_pool)
    .await
    .expect("forge a pre-0043 cross-tenant acceptance probe");

    PostgresStore::from_pool(upgrade_pool.clone())
        .migrate()
        .await
        .expect("upgrade version 42 to acceptance route reconciliation");

    let store = PostgresStore::from_pool(upgrade_pool.clone());
    let exact_proof: (Option<String>, Option<i64>, Option<Vec<u8>>, Option<i64>) = sqlx::query_as(
        "SELECT route_reservation_id,route_generation,route_fencing_token_hash,
                    connector_generation
             FROM job_acceptances
             WHERE job_id=$1",
    )
    .bind(fixtures[0].job_id)
    .fetch_one(&upgrade_pool)
    .await
    .expect("read exact tenant acceptance proof");
    let active_fence = Sha256::digest(b"fence-active").to_vec();
    assert_eq!(
        exact_proof,
        (
            Some("rrsv_accept_active".into()),
            Some(7),
            Some(active_fence.clone()),
            Some(1)
        )
    );
    let cross_tenant_proof: (Option<String>, Option<i64>, Option<Vec<u8>>, Option<i64>) =
        sqlx::query_as(
            "SELECT route_reservation_id,route_generation,route_fencing_token_hash,
                    connector_generation
             FROM job_acceptances
             WHERE job_id=$1",
        )
        .bind(fixtures[4].job_id)
        .fetch_one(&upgrade_pool)
        .await
        .expect("read cross-tenant legacy acceptance");
    assert_eq!(cross_tenant_proof, (None, None, None, None));
    let cross_tenant_key_change =
        sqlx::query("UPDATE job_acceptances SET environment_id=$1 WHERE job_id=$2")
            .bind(fixtures[1].environment_id)
            .bind(fixtures[4].job_id)
            .execute(&upgrade_pool)
            .await;
    assert!(
        cross_tenant_key_change.is_err(),
        "post-upgrade tenant-key writes must satisfy the composite job and agent fences"
    );
    let tenant_fences: Vec<(String, bool)> = sqlx::query_as(
        "SELECT conname,convalidated FROM pg_constraint
         WHERE conrelid='job_acceptances'::regclass
           AND conname IN ('job_acceptances_job_tenant_fk','job_acceptances_agent_tenant_fk')
         ORDER BY conname",
    )
    .fetch_all(&upgrade_pool)
    .await
    .expect("inspect acceptance tenant foreign keys");
    assert_eq!(
        tenant_fences,
        vec![
            ("job_acceptances_agent_tenant_fk".into(), false),
            ("job_acceptances_job_tenant_fk".into(), false),
        ]
    );

    for fixture in [fixtures[1], fixtures[2]] {
        let projection: (String, i64, bool, bool, String, bool, String, bool, String) =
            sqlx::query_as(
                "SELECT job.state,job.state_sequence,job.final_at IS NOT NULL,
                        job.delivery_uncertain_since IS NOT NULL,
                        attempt.state,attempt.final_at IS NOT NULL,
                        reservation.state,reservation.released_at IS NOT NULL,
                        destination.state
                 FROM jobs AS job
                 JOIN delivery_attempts AS attempt
                   ON attempt.workspace_id=job.workspace_id
                  AND attempt.environment_id=job.environment_id
                  AND attempt.job_id=job.id
                 JOIN route_reservations AS reservation
                   ON reservation.workspace_id=attempt.workspace_id
                  AND reservation.environment_id=attempt.environment_id
                  AND reservation.attempt_id=attempt.id
                 JOIN physical_destinations AS destination
                   ON destination.workspace_id=attempt.workspace_id
                  AND destination.environment_id=attempt.environment_id
                  AND destination.id=attempt.destination_id
                 WHERE job.id=$1 AND job.workspace_id=$2 AND job.environment_id=$3",
            )
            .bind(fixture.job_id)
            .bind(fixture.workspace_id)
            .bind(fixture.environment_id)
            .fetch_one(&upgrade_pool)
            .await
            .expect("read historical revoked acceptance projections");
        assert_eq!(
            projection,
            (
                "delivery_uncertain".into(),
                4,
                true,
                true,
                "delivery_uncertain".into(),
                true,
                "released".into(),
                true,
                "attention".into(),
            ),
            "revoked {} fixture must be terminalized consistently",
            fixture.name
        );
        let generation: Option<i64> =
            sqlx::query_scalar("SELECT connector_generation FROM job_acceptances WHERE job_id=$1")
                .bind(fixture.job_id)
                .fetch_one(&upgrade_pool)
                .await
                .expect("read revoked acceptance connector generation");
        assert_eq!(generation, Some(1));
        let event_payload: serde_json::Value = sqlx::query_scalar(
            "SELECT payload FROM job_events
             WHERE workspace_id=$1 AND environment_id=$2 AND job_id=$3",
        )
        .bind(fixture.workspace_id)
        .bind(fixture.environment_id)
        .bind(fixture.job_id)
        .fetch_one(&upgrade_pool)
        .await
        .expect("read historical uncertainty event payload");
        serde_json::from_value::<EventId>(event_payload["id"].clone())
            .expect("migration emitted a serde-compatible event ID");
        serde_json::from_value::<JobId>(event_payload["job_id"].clone())
            .expect("migration preserved a serde-compatible job ID");
        serde_json::from_value::<AgentId>(event_payload["agent_id"].clone())
            .expect("migration preserved a serde-compatible agent ID");
        let events = store
            .list_job_events(
                fixture
                    .workspace_id
                    .parse()
                    .expect("valid workspace fixture ID"),
                fixture
                    .environment_id
                    .parse()
                    .expect("valid environment fixture ID"),
                fixture.job_id.parse().expect("valid job fixture ID"),
            )
            .await
            .expect("migration event deserializes through PostgresStore");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].state, JobState::DeliveryUncertain);
        assert_eq!(events[0].reason, Some(JobFailureReason::AmbiguousHandoff));
        assert_eq!(
            events[0].agent_id.map(|id| id.to_string()),
            Some(fixture.agent_id.into())
        );
    }
    let active_projection: (String, bool, String, bool, String) = sqlx::query_as(
        "SELECT job.state,job.final_at IS NOT NULL,attempt.state,
                attempt.final_at IS NOT NULL,reservation.state
         FROM jobs AS job
         JOIN delivery_attempts AS attempt
           ON attempt.workspace_id=job.workspace_id
          AND attempt.environment_id=job.environment_id
          AND attempt.job_id=job.id
         JOIN route_reservations AS reservation
           ON reservation.workspace_id=attempt.workspace_id
          AND reservation.environment_id=attempt.environment_id
          AND reservation.attempt_id=attempt.id
         WHERE job.id=$1",
    )
    .bind(fixtures[0].job_id)
    .fetch_one(&upgrade_pool)
    .await
    .expect("read active connector projection");
    assert_eq!(
        active_projection,
        (
            "agent_accepted".into(),
            false,
            "queued_local".into(),
            false,
            "active".into()
        )
    );

    let legacy_before: (Option<String>, Option<i64>, Option<Vec<u8>>, Option<i64>) =
        sqlx::query_as(
            "SELECT route_reservation_id,route_generation,route_fencing_token_hash,
                    connector_generation
             FROM job_acceptances WHERE job_id=$1",
        )
        .bind(fixtures[3].job_id)
        .fetch_one(&upgrade_pool)
        .await
        .expect("read proofless legacy acceptance after migration");
    assert_eq!(legacy_before, (None, None, None, Some(1)));
    sqlx::query(
        "UPDATE job_acceptances SET connector_generation=NULL
         WHERE job_id=$1",
    )
    .bind(fixtures[3].job_id)
    .execute(&upgrade_pool)
    .await
    .expect("restore a fully legacy NULL acceptance shape");
    let legacy_reconciliation = store
        .reconcile_agent_acceptance(
            fixtures[3]
                .workspace_id
                .parse()
                .expect("valid workspace fixture ID"),
            fixtures[3]
                .environment_id
                .parse()
                .expect("valid environment fixture ID"),
            fixtures[3]
                .agent_id
                .parse()
                .expect("valid agent fixture ID"),
            fixtures[3].job_id.parse().expect("valid job fixture ID"),
            Uuid::parse_str(fixtures[3].lease_id).expect("valid lease fixture UUID"),
            "lease-token-legacy",
            "content-legacy",
            41,
            DeliveryAttemptProof {
                reservation_id: "rrsv_accept_legacy",
                generation: 7,
                fencing_token: "fence-legacy",
            },
        )
        .await
        .expect("reconcile exact legacy acceptance proof");
    assert_eq!(legacy_reconciliation, (true, false, false));
    let legacy_after: (String, i64, Vec<u8>, i64) = sqlx::query_as(
        "SELECT route_reservation_id,route_generation,route_fencing_token_hash,
                connector_generation
         FROM job_acceptances WHERE job_id=$1",
    )
    .bind(fixtures[3].job_id)
    .fetch_one(&upgrade_pool)
    .await
    .expect("read upgraded legacy acceptance proof");
    assert_eq!(
        legacy_after,
        (
            "rrsv_accept_legacy".into(),
            7,
            Sha256::digest(b"fence-legacy").to_vec(),
            1,
        )
    );

    // A post-upgrade N-1 writer still omits connector_generation. Against an
    // active connector the trigger fills the current generation while holding
    // the same connector-row lock used by revoke.
    let active_old_writer_job = "job_01J00000000000000000000105";
    sqlx::query(
        "INSERT INTO jobs
         (id,workspace_id,environment_id,printer_id,agent_id,payload,state,
          state_sequence,per_printer_sequence,expires_at,destination_id,route_id)
         VALUES ($1,$2,$3,$4,$5,'{}','waiting_for_agent',2,2,
                 now()+interval '1 hour',$6,$7)",
    )
    .bind(active_old_writer_job)
    .bind(fixtures[0].workspace_id)
    .bind(fixtures[0].environment_id)
    .bind(fixtures[0].printer_id)
    .bind(fixtures[0].agent_id)
    .bind("pdst_accept_active")
    .bind("rte_accept_active")
    .execute(&upgrade_pool)
    .await
    .expect("insert active old-writer job fixture");
    let mut active_old_writer = upgrade_pool
        .begin()
        .await
        .expect("begin active old-writer transaction");
    sqlx::query(
        "INSERT INTO job_acceptances
         (job_id,workspace_id,environment_id,agent_id,lease_id,lease_token_hash,
          content_sha256,local_sequence)
         VALUES ($1,$2,$3,$4,$5,$6,'content-active-old-writer',42)",
    )
    .bind(active_old_writer_job)
    .bind(fixtures[0].workspace_id)
    .bind(fixtures[0].environment_id)
    .bind(fixtures[0].agent_id)
    .bind(Uuid::parse_str("10000000-0000-0000-0000-000000000002").expect("valid lease UUID"))
    .bind(Sha256::digest(b"lease-token-active-old-writer").to_vec())
    .execute(&mut *active_old_writer)
    .await
    .expect("old-column insert is admitted for active connector");
    sqlx::query("UPDATE jobs SET state='agent_accepted',state_sequence=3 WHERE id=$1")
        .bind(active_old_writer_job)
        .execute(&mut *active_old_writer)
        .await
        .expect("simulate N-1 acceptance state transition");
    active_old_writer
        .commit()
        .await
        .expect("commit active N-1 acceptance");
    let active_old_writer_generation: Option<i64> =
        sqlx::query_scalar("SELECT connector_generation FROM job_acceptances WHERE job_id=$1")
            .bind(active_old_writer_job)
            .fetch_one(&upgrade_pool)
            .await
            .expect("read trigger-filled connector generation");
    assert_eq!(active_old_writer_generation, Some(1));

    // Insert-first ordering: an N-1 raw revoke must fail closed so it cannot
    // deny the credential while leaving accepted work stranded. The N path
    // sets the transaction-local bypass and owns the atomic projection sweep.
    let insert_first_revoke =
        sqlx::query("UPDATE node_connectors SET revoked_at=now() WHERE id=$1")
            .bind("ncon_accept_legacy")
            .execute(&upgrade_pool)
            .await;
    assert!(insert_first_revoke.is_err());
    let insert_first_unchanged: (bool, String) = sqlx::query_as(
        "SELECT connector.revoked_at IS NULL,job.state
         FROM node_connectors AS connector
         JOIN jobs AS job ON job.agent_id=connector.agent_id
          AND job.workspace_id=connector.workspace_id
          AND job.environment_id=connector.environment_id
         WHERE connector.id=$1 AND job.id=$2",
    )
    .bind("ncon_accept_legacy")
    .bind(fixtures[3].job_id)
    .fetch_one(&upgrade_pool)
    .await
    .expect("old revoke rollback preserves active connector and accepted job");
    assert_eq!(insert_first_unchanged, (true, "agent_accepted".into()));
    sqlx::query("DELETE FROM job_acceptances WHERE job_id=$1")
        .bind(fixtures[3].job_id)
        .execute(&upgrade_pool)
        .await
        .expect("remove completed rolling-upgrade fixture acceptance");
    sqlx::query("UPDATE jobs SET state='cancelled',final_at=now() WHERE id=$1")
        .bind(fixtures[3].job_id)
        .execute(&upgrade_pool)
        .await
        .expect("terminalize completed rolling-upgrade fixture job");
    sqlx::query("UPDATE node_connectors SET revoked_at=now() WHERE id=$1")
        .bind("ncon_accept_legacy")
        .execute(&upgrade_pool)
        .await
        .expect("old revoke is safe after accepted work is terminal");

    // Revoke-first ordering: the trigger aborts the old INSERT before its
    // simulated state transition, leaving no acceptance and a pre-accepted job.
    let revoke_first_job = "job_01J00000000000000000000405";
    sqlx::query(
        "INSERT INTO jobs
         (id,workspace_id,environment_id,printer_id,agent_id,payload,state,
          state_sequence,per_printer_sequence,expires_at,destination_id,route_id)
         VALUES ($1,$2,$3,$4,$5,'{}','waiting_for_agent',2,2,
                 now()+interval '1 hour',$6,$7)",
    )
    .bind(revoke_first_job)
    .bind(fixtures[3].workspace_id)
    .bind(fixtures[3].environment_id)
    .bind(fixtures[3].printer_id)
    .bind(fixtures[3].agent_id)
    .bind("pdst_accept_legacy")
    .bind("rte_accept_legacy")
    .execute(&upgrade_pool)
    .await
    .expect("insert revoke-first job fixture");
    let mut revoked_old_writer = upgrade_pool
        .begin()
        .await
        .expect("begin revoked old-writer transaction");
    let old_writer_after_revoke = sqlx::query(
        "INSERT INTO job_acceptances
         (job_id,workspace_id,environment_id,agent_id,lease_id,lease_token_hash,
          content_sha256,local_sequence)
         VALUES ($1,$2,$3,$4,$5,$6,'content-revoke-first',42)",
    )
    .bind(revoke_first_job)
    .bind(fixtures[3].workspace_id)
    .bind(fixtures[3].environment_id)
    .bind(fixtures[3].agent_id)
    .bind(Uuid::parse_str("40000000-0000-0000-0000-000000000002").expect("valid lease UUID"))
    .bind(Sha256::digest(b"lease-token-revoke-first").to_vec())
    .execute(&mut *revoked_old_writer)
    .await;
    assert!(
        old_writer_after_revoke.is_err(),
        "the acceptance trigger must reject an N-1 writer after revocation"
    );
    revoked_old_writer
        .rollback()
        .await
        .expect("roll back rejected old-writer transaction");
    let revoke_first_result: (String, i64) = sqlx::query_as(
        "SELECT job.state,count(acceptance.job_id)
         FROM jobs AS job
         LEFT JOIN job_acceptances AS acceptance ON acceptance.job_id=job.id
         WHERE job.id=$1 GROUP BY job.state",
    )
    .bind(revoke_first_job)
    .fetch_one(&upgrade_pool)
    .await
    .expect("inspect rejected revoke-first acceptance");
    assert_eq!(revoke_first_result, ("waiting_for_agent".into(), 0));

    let revoked_generation_before: i64 =
        sqlx::query_scalar("SELECT admission_generation FROM node_connectors WHERE id=$1")
            .bind("ncon_accept_revoked_agent")
            .fetch_one(&upgrade_pool)
            .await
            .expect("read backfilled revoked connector generation");
    assert_eq!(revoked_generation_before, 1);
    sqlx::query("UPDATE node_connectors SET revoked_at=NULL WHERE id=$1")
        .bind("ncon_accept_revoked_agent")
        .execute(&upgrade_pool)
        .await
        .expect("resurrect an old-style revoked connector");
    let revoked_generation_after: i64 =
        sqlx::query_scalar("SELECT admission_generation FROM node_connectors WHERE id=$1")
            .bind("ncon_accept_revoked_agent")
            .fetch_one(&upgrade_pool)
            .await
            .expect("read advanced connector generation");
    assert_eq!(revoked_generation_after, 2);
    let historical_acceptance_generation: Option<i64> =
        sqlx::query_scalar("SELECT connector_generation FROM job_acceptances WHERE job_id=$1")
            .bind(fixtures[1].job_id)
            .fetch_one(&upgrade_pool)
            .await
            .expect("read historical acceptance generation after resurrection");
    assert_eq!(historical_acceptance_generation, Some(1));

    let partial_proof = sqlx::query(
        "UPDATE job_acceptances
         SET route_generation=NULL
         WHERE job_id=$1",
    )
    .bind(fixtures[0].job_id)
    .execute(&upgrade_pool)
    .await;
    assert!(
        partial_proof.is_err(),
        "all-or-none route proof constraint must reject a missing generation"
    );
    let malformed_hash = sqlx::query(
        "UPDATE job_acceptances
         SET route_fencing_token_hash=decode(repeat('ff',31),'hex')
         WHERE job_id=$1",
    )
    .bind(fixtures[0].job_id)
    .execute(&upgrade_pool)
    .await;
    assert!(
        malformed_hash.is_err(),
        "route proof constraint must reject a tampered fencing hash length"
    );
    let invalid_connector_generation =
        sqlx::query("UPDATE job_acceptances SET connector_generation=0 WHERE job_id=$1")
            .bind(fixtures[0].job_id)
            .execute(&upgrade_pool)
            .await;
    assert!(
        invalid_connector_generation.is_err(),
        "connector generation constraint must reject a non-positive fence"
    );
    let preserved_exact_proof: (String, i64, Vec<u8>) = sqlx::query_as(
        "SELECT route_reservation_id,route_generation,route_fencing_token_hash
         FROM job_acceptances
         WHERE job_id=$1",
    )
    .bind(fixtures[0].job_id)
    .fetch_one(&upgrade_pool)
    .await
    .expect("failed tampering leaves the exact proof intact");
    assert_eq!(
        preserved_exact_proof,
        ("rrsv_accept_active".into(), 7, active_fence)
    );
    let upgraded_latest: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&upgrade_pool)
            .await
            .expect("read upgraded schema version");
    assert_eq!(upgraded_latest, 47);

    upgrade_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {upgrade_schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable upgrade schema");

    let fresh_schema = format!("piqae_acceptance_fresh_{}", ulid::Ulid::new()).to_ascii_lowercase();
    sqlx::query(&format!("CREATE SCHEMA {fresh_schema}"))
        .execute(&admin)
        .await
        .expect("create exact disposable fresh schema");
    let fresh_pool = schema_pool(&database_url, &fresh_schema).await;
    PostgresStore::from_pool(fresh_pool.clone())
        .migrate()
        .await
        .expect("application startup migrates an empty database to version 46");
    let fresh_latest: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&fresh_pool)
            .await
            .expect("read fresh schema version");
    assert_eq!(fresh_latest, 47);
    let fresh_columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT column_name,data_type
         FROM information_schema.columns
         WHERE table_schema=current_schema()
           AND table_name='job_acceptances'
           AND column_name IN (
               'route_reservation_id','route_generation','route_fencing_token_hash',
               'connector_generation'
           )
         ORDER BY column_name",
    )
    .fetch_all(&fresh_pool)
    .await
    .expect("inspect fresh acceptance route proof columns");
    assert_eq!(
        fresh_columns,
        vec![
            ("connector_generation".into(), "bigint".into()),
            ("route_fencing_token_hash".into(), "bytea".into()),
            ("route_generation".into(), "bigint".into()),
            ("route_reservation_id".into(), "text".into()),
        ]
    );
    let admission_generation: (String, String) = sqlx::query_as(
        "SELECT data_type,column_default
         FROM information_schema.columns
         WHERE table_schema=current_schema()
           AND table_name='node_connectors'
           AND column_name='admission_generation'",
    )
    .fetch_one(&fresh_pool)
    .await
    .expect("inspect fresh connector admission generation");
    assert_eq!(admission_generation.0, "bigint");
    assert_eq!(admission_generation.1, "1");
    let fresh_constraints: Vec<(String, String)> = sqlx::query_as(
        "SELECT conname,pg_get_constraintdef(oid)
         FROM pg_constraint
         WHERE conrelid='job_acceptances'::regclass
           AND conname IN (
               'job_acceptances_route_proof_complete',
               'job_acceptances_connector_generation_valid'
           )
         ORDER BY conname",
    )
    .fetch_all(&fresh_pool)
    .await
    .expect("inspect fresh acceptance fencing constraints");
    assert_eq!(fresh_constraints.len(), 2);
    assert!(fresh_constraints.iter().any(|(name, definition)| {
        name == "job_acceptances_route_proof_complete"
            && definition.contains("octet_length(route_fencing_token_hash) = 32")
    }));
    assert!(fresh_constraints.iter().any(|(name, definition)| {
        name == "job_acceptances_connector_generation_valid"
            && definition.contains("connector_generation > 0")
    }));
    let fresh_trigger_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_trigger
         WHERE tgrelid='node_connectors'::regclass
           AND tgname='node_connectors_advance_admission_generation'
           AND NOT tgisinternal",
    )
    .fetch_one(&fresh_pool)
    .await
    .expect("inspect fresh connector generation trigger");
    assert_eq!(fresh_trigger_count, 1);

    fresh_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {fresh_schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable fresh schema");
}

#[tokio::test]
async fn runtime_availability_upgrade_is_additive_and_tenant_isolated() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    let all = sqlx::migrate!("../../migrations/postgres");

    for mode in ["upgrade", "fresh"] {
        let schema = format!("piqae_runtime_{mode}_{}", ulid::Ulid::new()).to_ascii_lowercase();
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .expect("create exact disposable schema");
        let pool = schema_pool(&database_url, &schema).await;
        if mode == "upgrade" {
            let previous = Migrator {
                migrations: Cow::Owned(
                    all.iter()
                        .filter(|migration| migration.version < 41)
                        .cloned()
                        .collect(),
                ),
                ignore_missing: false,
                locking: true,
                no_tx: false,
            };
            previous.run(&pool).await.expect("apply version 40 schema");
        }
        PostgresStore::from_pool(pool.clone())
            .migrate()
            .await
            .expect("start storage on latest schema");

        for suffix in ["a", "b"] {
            sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
                .bind(format!("wsp_runtime_{suffix}"))
                .bind(format!("Runtime {suffix}"))
                .bind(format!("runtime-{mode}-{suffix}"))
                .execute(&pool)
                .await
                .expect("insert workspace fixture");
            sqlx::query(
                "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'test','Test')",
            )
            .bind(format!("env_runtime_{suffix}"))
            .bind(format!("wsp_runtime_{suffix}"))
            .execute(&pool)
            .await
            .expect("insert environment fixture");
            sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Node',$4,'linux','x86_64','test',1)")
                .bind(format!("agt_runtime_{suffix}"))
                .bind(format!("wsp_runtime_{suffix}"))
                .bind(format!("env_runtime_{suffix}"))
                .bind(format!("install-runtime-{suffix}"))
                .execute(&pool).await.expect("insert agent fixture");
        }
        sqlx::query("INSERT INTO node_runtime_observations (workspace_id,environment_id,id,agent_id,sequence,host_mode,availability_class,lifecycle_state,accepts_cloud_jobs,execution_budget_ms,wake_mechanisms,observed_at,fresh_until) VALUES ('wsp_runtime_a','env_runtime_a','nro_a','agt_runtime_a',1,'embedded_application','background_opportunistic','background',true,30000,ARRAY['apns_background'],now(),now()+interval '60 seconds')")
            .execute(&pool).await.expect("insert tenant runtime observation");
        sqlx::query("INSERT INTO node_wake_hints (workspace_id,environment_id,id,agent_id,idempotency_key,reason,status,requested_at,expires_at) VALUES ('wsp_runtime_a','env_runtime_a','wkh_a','agt_runtime_a','wake-key-a','job_available','pending',now(),now()+interval '5 minutes')")
            .execute(&pool).await.expect("insert tenant wake hint");
        let cross_tenant_runtime = sqlx::query("INSERT INTO node_runtime_observations (workspace_id,environment_id,id,agent_id,sequence,host_mode,availability_class,lifecycle_state,accepts_cloud_jobs,wake_mechanisms,observed_at,fresh_until) VALUES ('wsp_runtime_b','env_runtime_b','nro_cross','agt_runtime_a',2,'embedded_application','foreground_only','foreground',false,'{}',now(),now()+interval '60 seconds')")
            .execute(&pool).await;
        assert!(
            cross_tenant_runtime.is_err(),
            "the composite agent foreign key must reject cross-tenant runtime state"
        );
        let cross_tenant_hint = sqlx::query("INSERT INTO node_wake_hints (workspace_id,environment_id,id,agent_id,idempotency_key,reason,status,requested_at,expires_at) VALUES ('wsp_runtime_b','env_runtime_b','wkh_cross','agt_runtime_a','wake-cross','job_available','pending',now(),now()+interval '5 minutes')")
            .execute(&pool).await;
        assert!(
            cross_tenant_hint.is_err(),
            "the composite agent foreign key must reject cross-tenant wake state"
        );
        let other_runtime: i64 = sqlx::query_scalar("SELECT count(*) FROM node_runtime_observations WHERE workspace_id='wsp_runtime_b' AND environment_id='env_runtime_b' AND agent_id='agt_runtime_a'")
            .fetch_one(&pool).await.expect("probe other tenant runtime");
        let other_hints: i64 = sqlx::query_scalar("SELECT count(*) FROM node_wake_hints WHERE workspace_id='wsp_runtime_b' AND environment_id='env_runtime_b' AND agent_id='agt_runtime_a'")
            .fetch_one(&pool).await.expect("probe other tenant hints");
        assert_eq!((other_runtime, other_hints), (0, 0));

        pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .expect("drop exact disposable schema");
    }
}

#[tokio::test]
async fn semantic_capabilities_upgrade_is_tenant_scoped_and_backfilled() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let schema = format!("piqae_semantic_migration_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create disposable schema");
    let pool = schema_pool(&database_url, &schema).await;
    let all = sqlx::migrate!("../../migrations/postgres");
    let previous = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 31)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    previous.run(&pool).await.expect("apply version 30 schema");
    for suffix in ["a", "b"] {
        sqlx::query("INSERT INTO workspaces (id, name, slug) VALUES ($1, $2, $3)")
            .bind(format!("wsp_{suffix}"))
            .bind(format!("Workspace {suffix}"))
            .bind(format!("workspace-{suffix}"))
            .execute(&pool)
            .await
            .expect("insert workspace");
        sqlx::query(
            "INSERT INTO environments (id, workspace_id, kind, name) VALUES ($1,$2,'test','Test')",
        )
        .bind(format!("env_{suffix}"))
        .bind(format!("wsp_{suffix}"))
        .execute(&pool)
        .await
        .expect("insert environment");
        sqlx::query("INSERT INTO agents (id, workspace_id, environment_id, name, installation_id, os, architecture, version, protocol_version) VALUES ($1,$2,$3,'Node',$4,'linux','x86_64','test',1)")
            .bind(format!("agt_{suffix}"))
            .bind(format!("wsp_{suffix}"))
            .bind(format!("env_{suffix}"))
            .bind(format!("install-{suffix}"))
            .execute(&pool).await.expect("insert agent");
        sqlx::query("INSERT INTO printers (id, workspace_id, environment_id, agent_id, native_id, name) VALUES ($1,$2,$3,$4,$5,'Printer')")
            .bind(format!("prt_{suffix}"))
            .bind(format!("wsp_{suffix}"))
            .bind(format!("env_{suffix}"))
            .bind(format!("agt_{suffix}"))
            .bind(format!("native-{suffix}"))
            .execute(&pool).await.expect("insert printer");
    }
    PostgresStore::from_pool(pool.clone())
        .migrate()
        .await
        .expect("upgrade to semantic capability schema");
    let defaults: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM printers WHERE semantic_capabilities = '{}'::jsonb",
    )
    .fetch_one(&pool)
    .await
    .expect("verify safe backfill");
    assert_eq!(defaults, 2);
    sqlx::query("UPDATE printers SET semantic_capabilities = '{\"facets\":{\"media.sensing\":[\"gap\"]}}'::jsonb WHERE id = 'prt_a' AND workspace_id = 'wsp_a' AND environment_id = 'env_a'")
        .execute(&pool).await.expect("update exact tenant printer");
    let untouched: serde_json::Value = sqlx::query_scalar(
        "SELECT semantic_capabilities FROM printers WHERE id = 'prt_b' AND workspace_id = 'wsp_b' AND environment_id = 'env_b'",
    )
    .fetch_one(&pool).await.expect("read other tenant printer");
    assert_eq!(untouched, serde_json::json!({}));
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}

async fn schema_pool(database_url: &str, schema: &str) -> PgPool {
    let schema = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connection, _metadata| {
            let statement = format!("SET search_path TO {schema}");
            Box::pin(async move {
                sqlx::query(&statement).execute(connection).await?;
                Ok(())
            })
        })
        .connect(database_url)
        .await
        .expect("connect disposable PostgreSQL schema")
}

async fn wait_for_database_blocker(pool: &PgPool, pid: i32) {
    for _ in 0..50 {
        let blocked: bool = sqlx::query_scalar(
            "SELECT cardinality(pg_blocking_pids($1)) > 0
             AND EXISTS (
                 SELECT 1 FROM pg_stat_activity
                 WHERE pid = $1 AND wait_event_type = 'Lock'
             )",
        )
        .bind(pid)
        .fetch_one(pool)
        .await
        .expect("observe PostgreSQL lock waiter");
        if blocked {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("PostgreSQL did not report the expected blocked transaction");
}

#[tokio::test]
async fn postgres_reported_complete_billing_upgrades_from_previous_schema() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };

    let schema = format!("piqae_migration_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create disposable schema");
    let pool = schema_pool(&database_url, &schema).await;

    let all = sqlx::migrate!("../../migrations/postgres");
    let previous = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 17)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    previous
        .run(&pool)
        .await
        .expect("apply previous schema version");

    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("upgrade to latest schema");

    let latest: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read latest schema version");
    assert_eq!(latest, 47);
    let billable_index: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('usage_one_billable_print_per_job_idx')::text")
            .fetch_one(&pool)
            .await
            .expect("inspect billable usage index");
    assert_eq!(
        billable_index.as_deref(),
        Some("usage_one_billable_print_per_job_idx")
    );

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn documents_migrate_and_enforce_tenant_scoped_references() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let schema = format!("piqae_documents_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create disposable schema");
    let pool = schema_pool(&database_url, &schema).await;
    let store = PostgresStore::from_pool(pool.clone());
    store
        .migrate()
        .await
        .expect("migrate empty database through documents");
    let printpacket_tables: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM unnest(ARRAY[
             'printpacket_resources',
             'printpacket_resource_references'
         ]) AS name
         WHERE to_regclass(current_schema() || '.' || name) IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("inspect canonical PrintPacket tables");
    assert_eq!(printpacket_tables, 2);
    let predecessor_tables: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_class relation
         JOIN pg_namespace namespace ON namespace.oid=relation.relnamespace
         WHERE namespace.nspname=current_schema()
           AND relation.relname LIKE 'business\\_document%' ESCAPE '\\'",
    )
    .fetch_one(&pool)
    .await
    .expect("inspect removed pre-release table names");
    assert_eq!(predecessor_tables, 0);
    for suffix in ["a", "b"] {
        sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
            .bind(format!("wsp_doc_{suffix}"))
            .bind(suffix)
            .bind(format!("documents-{suffix}"))
            .execute(&pool)
            .await
            .expect("workspace fixture");
        sqlx::query(
            "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'live','Live')",
        )
        .bind(format!("env_doc_{suffix}"))
        .bind(format!("wsp_doc_{suffix}"))
        .execute(&pool)
        .await
        .expect("environment fixture");
    }
    let ciphertext = b"authenticated-ciphertext-not-plaintext";
    sqlx::query(
        "INSERT INTO document_templates
        (id,workspace_id,environment_id,name,draft_ciphertext,draft_sha256)
        VALUES ('dtpl_doc_a','wsp_doc_a','env_doc_a','Receipt',$1,$2)",
    )
    .bind(ciphertext.as_slice())
    .bind("a".repeat(64))
    .execute(&pool)
    .await
    .expect("tenant document template");
    let cross_tenant_revision = sqlx::query("INSERT INTO document_template_revisions
        (id,workspace_id,environment_id,template_id,revision,spec_ciphertext,spec_sha256,renderer_profile)
        VALUES ('drev_cross','wsp_doc_b','env_doc_b','dtpl_doc_a',1,$1,$2,'printpacket/v1')")
        .bind(ciphertext.as_slice()).bind("a".repeat(64)).execute(&pool).await;
    assert!(
        cross_tenant_revision.is_err(),
        "composite tenant foreign key must reject probing"
    );
    let removed_format = sqlx::query(
        "INSERT INTO document_template_revisions
         (id,workspace_id,environment_id,template_id,revision,spec_ciphertext,spec_sha256,renderer_profile)
         VALUES ('drev_removed','wsp_doc_a','env_doc_a','dtpl_doc_a',2,$1,$2,'piqae.business-document/v1')",
    )
    .bind(ciphertext.as_slice())
    .bind("a".repeat(64))
    .execute(&pool)
    .await;
    assert!(
        removed_format.is_err(),
        "the pre-release format identifier must fail the database constraint"
    );
    sqlx::query(
        "INSERT INTO document_template_revisions
         (id,workspace_id,environment_id,template_id,revision,spec_ciphertext,spec_sha256,renderer_profile)
         VALUES ('drev_doc_a','wsp_doc_a','env_doc_a','dtpl_doc_a',1,$1,$2,'printpacket/v1')",
    )
    .bind(ciphertext.as_slice())
    .bind("a".repeat(64))
    .execute(&pool)
    .await
    .expect("revision fixture");
    sqlx::query(
        "INSERT INTO document_renders
         (id,workspace_id,environment_id,template_revision_id,input_ciphertext,input_sha256,state,
          artifact_object_key_ciphertext,artifact_sha256,artifact_byte_length,artifact_media_type,
          idempotency_key,request_sha256)
         VALUES ('drnd_doc_a','wsp_doc_a','env_doc_a','drev_doc_a',$1,$2,'completed',$1,$2,123,
          'application/pdf','render-key-a',$2)",
    )
    .bind(ciphertext.as_slice())
    .bind("a".repeat(64))
    .execute(&pool)
    .await
    .expect("completed render fixture");
    let resource_digest = "b".repeat(64);
    for suffix in ["a", "b"] {
        sqlx::query(
            "INSERT INTO printpacket_resources
             (workspace_id,environment_id,digest,media_type,byte_length)
             VALUES ($1,$2,$3,'image/jpeg',128)",
        )
        .bind(format!("wsp_doc_{suffix}"))
        .bind(format!("env_doc_{suffix}"))
        .bind(&resource_digest)
        .execute(&pool)
        .await
        .expect("tenant PrintPacket resource fixture");
    }
    let cross_tenant_resource_link = sqlx::query(
        "INSERT INTO printpacket_resource_references
         (workspace_id,environment_id,render_id,resource_digest)
         VALUES ('wsp_doc_b','env_doc_b','drnd_doc_a',$1)",
    )
    .bind(&resource_digest)
    .execute(&pool)
    .await;
    assert!(
        cross_tenant_resource_link.is_err(),
        "resource references must not cross a tenant boundary"
    );
    sqlx::query(
        "INSERT INTO uploads
         (id,workspace_id,environment_id,object_key,media_type,expected_sha256,expected_bytes,state,
          expires_at,completed_at,source_document_render_id,acquisition_sha256)
         VALUES ('dua_doc_a','wsp_doc_a','env_doc_a','objects/render-a','application/pdf',$1,123,
          'complete',now()+interval '1 hour',now(),'drnd_doc_a',$1)",
    )
    .bind("a".repeat(64))
    .execute(&pool)
    .await
    .expect("completed render artifact fixture");
    sqlx::query(
        "INSERT INTO agents
         (id,workspace_id,environment_id,name,installation_id,os,architecture,version,protocol_version)
         VALUES ('agt_doc_a','wsp_doc_a','env_doc_a','Fixture','install-doc-a','linux','x86_64','test',1)",
    )
    .execute(&pool)
    .await
    .expect("agent fixture");
    sqlx::query(
        "INSERT INTO printers
         (id,workspace_id,environment_id,agent_id,native_id,name)
         VALUES ('prn_doc_a','wsp_doc_a','env_doc_a','agt_doc_a','virtual-doc-a','Virtual')",
    )
    .execute(&pool)
    .await
    .expect("printer fixture");
    sqlx::query(
        "INSERT INTO jobs
         (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at)
         VALUES ('job_doc_a','wsp_doc_a','env_doc_a','prn_doc_a','agt_doc_a',
          '{\"content\":{\"type\":\"upload\",\"upload_id\":\"dua_doc_a\"}}'::jsonb,
          'registered',1,now()+interval '2 hours')",
    )
    .execute(&pool)
    .await
    .expect("artifact-backed job fixture");
    let artifact_edge: (String, String) = sqlx::query_as(
        "SELECT upload_id,render_id FROM document_artifact_job_references
         WHERE workspace_id='wsp_doc_a' AND environment_id='env_doc_a' AND job_id='job_doc_a'",
    )
    .fetch_one(&pool)
    .await
    .expect("trigger registered exact upload and render identifiers");
    assert_eq!(artifact_edge, ("dua_doc_a".into(), "drnd_doc_a".into()));
    let plaintext_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM document_templates
        WHERE convert_from(draft_ciphertext, 'UTF8') LIKE '%spec_version%'",
    )
    .fetch_one(&pool)
    .await
    .expect("inspect encrypted persistence");
    assert_eq!(plaintext_rows, 0);
    let artifact_reference_constraints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_constraint c
         JOIN pg_class t ON t.oid=c.conrelid
         JOIN pg_namespace n ON n.oid=t.relnamespace
         WHERE n.nspname=current_schema()
           AND t.relname='document_artifact_job_references'
           AND c.contype IN ('p','f','c')",
    )
    .fetch_one(&pool)
    .await
    .expect("inspect artifact ownership constraints");
    assert_eq!(artifact_reference_constraints, 6);
    let active_reference_index: bool = sqlx::query_scalar(
        // Resolved by name rather than scanned from pg_indexes: a catalog scan
        // can trip over a schema another test is dropping concurrently and fail
        // with "could not open relation with OID".
        "SELECT COALESCE(
             pg_get_indexdef(to_regclass(
                 current_schema() || '.document_artifact_active_references_idx'
             )) LIKE '%released_at IS NULL%',
             false
         )",
    )
    .fetch_one(&pool)
    .await
    .expect("inspect active artifact reference index");
    assert!(active_reference_index);
    let stable_document_indexes: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM unnest(ARRAY[
             'document_templates_tenant_created_idx',
             'document_renders_tenant_created_idx'
         ]) AS name
         WHERE pg_get_indexdef(to_regclass(current_schema() || '.' || name))
               LIKE '%created_at DESC, id%'",
    )
    .fetch_one(&pool)
    .await
    .expect("inspect stable tenant pagination indexes");
    assert_eq!(stable_document_indexes, 2);
    let render_revision_delete_action: String = sqlx::query_scalar(
        "SELECT confdeltype::text FROM pg_constraint
          WHERE conrelid='document_renders'::regclass
            AND confrelid='document_template_revisions'::regclass
            AND contype='f'",
    )
    .fetch_one(&pool)
    .await
    .expect("inspect render revision delete policy");
    assert_eq!(render_revision_delete_action, "c");
    let latest: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read schema version");
    assert_eq!(latest, 47);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn document_key_retirement_waits_for_every_retained_ciphertext() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let schema = format!("piqae_document_keys_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create disposable schema");
    let pool = schema_pool(&database_url, &schema).await;
    let store = PostgresStore::from_pool(pool.clone());
    store.migrate().await.expect("migrate empty database");
    sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ('wsp_01J00000000000000000000000','Keys','keys-a')")
        .execute(&pool)
        .await
        .expect("workspace fixture");
    sqlx::query(
        "INSERT INTO environments (id,workspace_id,kind,name)
         VALUES ('env_01J00000000000000000000001','wsp_01J00000000000000000000000','live','Live')",
    )
    .execute(&pool)
    .await
    .expect("environment fixture");
    sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ('wsp_01J00000000000000000000002','Other','keys-b')")
        .execute(&pool)
        .await
        .expect("other workspace fixture");
    sqlx::query(
        "INSERT INTO environments (id,workspace_id,kind,name)
         VALUES ('env_01J00000000000000000000003','wsp_01J00000000000000000000002','live','Live')",
    )
    .execute(&pool)
    .await
    .expect("other environment fixture");
    // A pre-key-id ciphertext must remain attributed to legacy-v1.
    sqlx::query(
        "INSERT INTO document_templates
         (id,workspace_id,environment_id,name,draft_ciphertext,draft_sha256)
         VALUES ('dtpl_key_a','wsp_01J00000000000000000000000','env_01J00000000000000000000001','Receipt',$1,$2)",
    )
    .bind(vec![7_u8; 29])
    .bind("a".repeat(64))
    .execute(&pool)
    .await
    .expect("legacy encrypted document");
    let missing = store
        .update_document_template_draft(
            "wsp_01J00000000000000000000000"
                .parse()
                .expect("valid workspace"),
            "env_01J00000000000000000000001"
                .parse()
                .expect("valid environment"),
            "dtpl_missing",
            &[9_u8; 29],
            &"b".repeat(64),
        )
        .await;
    assert!(matches!(
        missing,
        Err(piqae_storage_postgres::StorageError::NotFound)
    ));
    let cross_tenant = store
        .update_document_template_draft(
            "wsp_01J00000000000000000000002"
                .parse()
                .expect("valid workspace"),
            "env_01J00000000000000000000003"
                .parse()
                .expect("valid environment"),
            "dtpl_key_a",
            &[9_u8; 29],
            &"b".repeat(64),
        )
        .await;
    assert!(matches!(
        cross_tenant,
        Err(piqae_storage_postgres::StorageError::NotFound)
    ));
    sqlx::query("UPDATE document_templates SET state='published' WHERE id='dtpl_key_a'")
        .execute(&pool)
        .await
        .expect("published template fixture");
    let conflicting = store
        .update_document_template_draft(
            "wsp_01J00000000000000000000000"
                .parse()
                .expect("valid workspace"),
            "env_01J00000000000000000000001"
                .parse()
                .expect("valid environment"),
            "dtpl_key_a",
            &[9_u8; 29],
            &"b".repeat(64),
        )
        .await;
    assert!(matches!(
        conflicting,
        Err(piqae_storage_postgres::StorageError::IdempotencyConflict)
    ));
    let references: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM document_encryption_key_references WHERE key_id='legacy-v1'",
    )
    .fetch_one(&pool)
    .await
    .expect("audit legacy references");
    assert_eq!(references, 1);
    let referenced_delete =
        sqlx::query("DELETE FROM document_encryption_keys WHERE key_id='legacy-v1'")
            .execute(&pool)
            .await;
    assert!(
        referenced_delete.is_err(),
        "retained ciphertext must block key deletion"
    );
    assert!(
        store
            .verify_document_encryption_keyring(["new-key"])
            .await
            .is_err(),
        "startup must reject a keyring missing retained work"
    );
    store
        .verify_document_encryption_keyring(["legacy-v1"])
        .await
        .expect("complete keyring admits retained work");
    store
        .configure_document_encryption_keys("new-key", ["legacy-v1", "new-key"])
        .await
        .expect("activate new generation while retaining old decryption key");
    store
        .verify_persisted_document_encryption_keyring("new-key", ["legacy-v1", "new-key"])
        .await
        .expect("maintenance keyring matches durable lifecycle without mutation");
    assert!(
        store
            .verify_persisted_document_encryption_keyring("legacy-v1", ["legacy-v1", "new-key"],)
            .await
            .is_err(),
        "maintenance cannot operate with a stale configured active generation"
    );
    assert!(
        store
            .configure_document_encryption_keys("legacy-v1", ["legacy-v1", "new-key"])
            .await
            .is_err(),
        "a stale process cannot roll the active generation backwards"
    );
    let retirement = sqlx::query(
        "UPDATE document_encryption_keys SET lifecycle_state='retired'
         WHERE key_id='legacy-v1'",
    )
    .execute(&pool)
    .await;
    assert!(
        retirement.is_err(),
        "retained ciphertext must block retirement"
    );
    let records = store
        .document_ciphertexts_for_rewrap("legacy-v1", 10)
        .await
        .expect("claim bounded rewrap batch");
    assert_eq!(records.len(), 1);
    let mut replacement = b"PDOC\x02\x07new-key".to_vec();
    replacement.extend([8_u8; 40]);
    assert!(
        store
            .rewrap_document_ciphertext(&records[0], &replacement)
            .await
            .expect("compare-and-swap retained ciphertext")
    );
    assert!(
        !store
            .rewrap_document_ciphertext(&records[0], &replacement)
            .await
            .expect("restart-safe stale compare-and-swap"),
        "a restarted batch must not overwrite its completed replacement"
    );
    assert_eq!(
        store
            .document_encryption_key_reference_count("legacy-v1")
            .await
            .expect("verify drained key generation"),
        0
    );
    sqlx::query(
        "UPDATE document_encryption_keys SET lifecycle_state='retired'
         WHERE key_id='legacy-v1'",
    )
    .execute(&pool)
    .await
    .expect("zero references permit retirement");

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn node_connector_upgrade_backfills_without_cross_tenant_merging() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let schema = format!("piqae_connector_migration_{}", ulid::Ulid::new()).to_ascii_lowercase();
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    sqlx::query(&format!("CREATE SCHEMA {schema}"))
        .execute(&admin)
        .await
        .expect("create disposable schema");
    let pool = schema_pool(&database_url, &schema).await;
    let all = sqlx::migrate!("../../migrations/postgres");
    let previous = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 19)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    previous
        .run(&pool)
        .await
        .expect("apply schema through 0018");
    for suffix in ["a", "b"] {
        sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
            .bind(format!("wsp_{suffix}"))
            .bind(suffix)
            .bind(format!("migration-{suffix}"))
            .execute(&pool)
            .await
            .expect("workspace");
        sqlx::query(
            "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'live','Live')",
        )
        .bind(format!("env_{suffix}"))
        .bind(format!("wsp_{suffix}"))
        .execute(&pool)
        .await
        .expect("environment");
        sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,public_key,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Node','historical-collision',$4,'test','test','1',1)")
            .bind(format!("agt_{suffix}")).bind(format!("wsp_{suffix}"))
            .bind(format!("env_{suffix}")).bind(vec![suffix.as_bytes()[0]])
            .execute(&pool).await.expect("legacy agent");
    }
    PostgresStore::from_pool(pool.clone())
        .migrate()
        .await
        .expect("upgrade through 0022");
    let installations: i64 = sqlx::query_scalar("SELECT count(*) FROM node_installations")
        .fetch_one(&pool)
        .await
        .expect("installation count");
    let connectors: i64 = sqlx::query_scalar("SELECT count(*) FROM node_connectors")
        .fetch_one(&pool)
        .await
        .expect("connector count");
    assert_eq!((installations, connectors), (2, 2));
    let connector_mappings: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT connector.agent_id,
                connector.workspace_id,
                connector.environment_id,
                connector.installation_id,
                installation.installation_key
         FROM node_connectors connector
         JOIN node_installations installation
           ON installation.id = connector.installation_id
         ORDER BY connector.agent_id",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect tenant connector installation mapping");
    assert_eq!(
        connector_mappings,
        vec![
            (
                "agt_a".into(),
                "wsp_a".into(),
                "env_a".into(),
                "ninst_agt_a".into(),
                "legacy:agt_a".into(),
            ),
            (
                "agt_b".into(),
                "wsp_b".into(),
                "env_b".into(),
                "ninst_agt_b".into(),
                "legacy:agt_b".into(),
            ),
        ],
        "colliding tenant-local legacy installation IDs must remain distinct"
    );
    let connector_columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name
         FROM information_schema.columns
         WHERE table_schema = current_schema()
           AND table_name = 'node_connectors'
           AND column_name IN (
             'installation_id', 'workspace_id', 'environment_id', 'agent_id'
           )
         ORDER BY column_name",
    )
    .fetch_all(&pool)
    .await
    .expect("inspect connector tenant columns");
    assert_eq!(
        connector_columns,
        vec![
            "agent_id",
            "environment_id",
            "installation_id",
            "workspace_id"
        ]
    );
    let cross_tenant: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM node_connectors
         WHERE (workspace_id = 'wsp_a' AND agent_id = 'agt_b')
            OR (workspace_id = 'wsp_b' AND agent_id = 'agt_a')",
    )
    .fetch_one(&pool)
    .await
    .expect("tenant isolation query");
    assert_eq!(cross_tenant, 0);
    sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_a','env_a','agt_a','cek_a','RSA-OAEP-256',$1)")
        .bind("A".repeat(128)).execute(&pool).await.expect("tenant key");
    let cross_tenant_key = sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_a','env_a','agt_b','cek_probe','RSA-OAEP-256',$1)")
        .bind("B".repeat(128)).execute(&pool).await;
    assert!(
        cross_tenant_key.is_err(),
        "composite agent foreign key must reject cross-tenant key registration"
    );
    sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_b','env_b','agt_b','cek_ecdh','ECDH-P256-HKDF-SHA256',$1)")
        .bind("C".repeat(122))
        .execute(&pool)
        .await
        .expect("tenant-scoped P-256 encryption key");
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn agent_health_migrates_empty_and_previous_schemas_with_tenant_fencing() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    let all = sqlx::migrate!("../../migrations/postgres");

    let empty_schema = format!("piqae_health_empty_{}", ulid::Ulid::new()).to_ascii_lowercase();
    sqlx::query(&format!("CREATE SCHEMA {empty_schema}"))
        .execute(&admin)
        .await
        .expect("create empty-database schema");
    let empty_pool = schema_pool(&database_url, &empty_schema).await;
    all.run(&empty_pool).await.expect("migrate empty database");
    let latest: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&empty_pool)
        .await
        .expect("read empty-database schema version");
    assert_eq!(latest, 47);
    empty_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {empty_schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact empty-database schema");

    let upgrade_schema = format!("piqae_health_upgrade_{}", ulid::Ulid::new()).to_ascii_lowercase();
    sqlx::query(&format!("CREATE SCHEMA {upgrade_schema}"))
        .execute(&admin)
        .await
        .expect("create upgrade schema");
    let upgrade_pool = schema_pool(&database_url, &upgrade_schema).await;
    let previous = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 26)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    previous.run(&upgrade_pool).await.expect("apply schema 25");
    for suffix in ["a", "b"] {
        sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
            .bind(format!("wsp_health_{suffix}"))
            .bind(suffix)
            .bind(format!("health-{suffix}"))
            .execute(&upgrade_pool)
            .await
            .expect("workspace");
        sqlx::query(
            "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'live','Live')",
        )
        .bind(format!("env_health_{suffix}"))
        .bind(format!("wsp_health_{suffix}"))
        .execute(&upgrade_pool)
        .await
        .expect("environment");
        sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Node',$4,'test','test','1',1)")
            .bind(format!("agt_health_{suffix}"))
            .bind(format!("wsp_health_{suffix}"))
            .bind(format!("env_health_{suffix}"))
            .bind(format!("installation-health-{suffix}"))
            .execute(&upgrade_pool).await.expect("legacy agent");
    }
    all.run(&upgrade_pool)
        .await
        .expect("upgrade schema 25 to 26");
    let own_update = sqlx::query(
        "UPDATE agents SET executor_crashes = 2, last_error_code = 'executor_crashed'
         WHERE id = 'agt_health_a' AND workspace_id = 'wsp_health_a'
           AND environment_id = 'env_health_a'",
    )
    .execute(&upgrade_pool)
    .await
    .expect("tenant health update");
    assert_eq!(own_update.rows_affected(), 1);
    let cross_tenant_probe = sqlx::query(
        "UPDATE agents SET executor_crashes = 99
         WHERE id = 'agt_health_b' AND workspace_id = 'wsp_health_a'
           AND environment_id = 'env_health_a'",
    )
    .execute(&upgrade_pool)
    .await
    .expect("cross-tenant probe");
    assert_eq!(cross_tenant_probe.rows_affected(), 0);
    let other_count: i64 =
        sqlx::query_scalar("SELECT executor_crashes FROM agents WHERE id = 'agt_health_b'")
            .fetch_one(&upgrade_pool)
            .await
            .expect("other tenant health");
    assert_eq!(other_count, 0);
    sqlx::query("INSERT INTO node_diagnostics (request_id, workspace_id, environment_id, agent_id, state) VALUES ('diag_a','wsp_health_a','env_health_a','agt_health_a','requested')")
        .execute(&upgrade_pool).await.expect("tenant diagnostic request");
    let cross_tenant_insert = sqlx::query("INSERT INTO node_diagnostics (request_id, workspace_id, environment_id, agent_id, state) VALUES ('diag_cross','wsp_health_a','env_health_a','agt_health_b','requested')")
        .execute(&upgrade_pool).await;
    assert!(
        cross_tenant_insert.is_err(),
        "composite agent foreign key must reject a cross-tenant diagnostic"
    );
    let cross_tenant_report = sqlx::query("UPDATE node_diagnostics SET state = 'complete', report = '{}'::jsonb WHERE request_id = 'diag_a' AND workspace_id = 'wsp_health_b' AND environment_id = 'env_health_b' AND agent_id = 'agt_health_b'")
        .execute(&upgrade_pool).await.expect("cross-tenant diagnostic probe");
    assert_eq!(cross_tenant_report.rows_affected(), 0);
    sqlx::query(
        "INSERT INTO node_diagnostics
             (request_id, workspace_id, environment_id, agent_id, state, expires_at)
         SELECT 'diag_expired_' || value, 'wsp_health_a', 'env_health_a',
                'agt_health_a', 'requested', now() - interval '1 minute'
         FROM generate_series(1, 1001) value",
    )
    .execute(&upgrade_pool)
    .await
    .expect("expired diagnostic fixtures");
    let store = PostgresStore::from_pool(upgrade_pool.clone());
    let first_purge = store
        .purge_expired_authentication_state()
        .await
        .expect("first bounded diagnostic purge");
    assert_eq!(first_purge.node_diagnostics, 1000);
    let expired_remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM node_diagnostics WHERE expires_at <= now()")
            .fetch_one(&upgrade_pool)
            .await
            .expect("count remaining expired diagnostics");
    assert_eq!(expired_remaining, 1);
    let second_purge = store
        .purge_expired_authentication_state()
        .await
        .expect("second bounded diagnostic purge");
    assert_eq!(second_purge.node_diagnostics, 1);

    upgrade_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {upgrade_schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact upgrade schema");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn content_encryption_key_algorithm_migrates_fresh_and_legacy_schemas() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    let all = sqlx::migrate!("../../migrations/postgres");

    let empty_schema = format!("piqae_cek_empty_{}", ulid::Ulid::new()).to_ascii_lowercase();
    sqlx::query(&format!("CREATE SCHEMA {empty_schema}"))
        .execute(&admin)
        .await
        .expect("create empty-database schema");
    let empty_pool = schema_pool(&database_url, &empty_schema).await;
    PostgresStore::from_pool(empty_pool.clone())
        .migrate()
        .await
        .expect("application startup migrates an empty database");
    let empty_latest: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&empty_pool)
        .await
        .expect("read empty-database schema version");
    assert_eq!(empty_latest, 47);
    empty_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {empty_schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact empty-database schema");

    let upgrade_schema = format!("piqae_cek_upgrade_{}", ulid::Ulid::new()).to_ascii_lowercase();
    sqlx::query(&format!("CREATE SCHEMA {upgrade_schema}"))
        .execute(&admin)
        .await
        .expect("create upgrade schema");
    let upgrade_pool = schema_pool(&database_url, &upgrade_schema).await;
    let before_algorithm_expansion = Migrator {
        migrations: Cow::Owned(
            all.iter()
                .filter(|migration| migration.version < 24)
                .cloned()
                .collect(),
        ),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    before_algorithm_expansion
        .run(&upgrade_pool)
        .await
        .expect("apply schema through 0023");

    for suffix in ["a", "b"] {
        sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
            .bind(format!("wsp_cek_{suffix}"))
            .bind(format!("CEK tenant {suffix}"))
            .bind(format!("cek-tenant-{suffix}"))
            .execute(&upgrade_pool)
            .await
            .expect("insert workspace");
        sqlx::query(
            "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'live','Live')",
        )
        .bind(format!("env_cek_{suffix}"))
        .bind(format!("wsp_cek_{suffix}"))
        .execute(&upgrade_pool)
        .await
        .expect("insert environment");
        sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,os,architecture,version,protocol_version) VALUES ($1,$2,$3,'Node',$4,'test','test','1',1)")
            .bind(format!("agt_cek_{suffix}"))
            .bind(format!("wsp_cek_{suffix}"))
            .bind(format!("env_cek_{suffix}"))
            .bind(format!("installation-cek-{suffix}"))
            .execute(&upgrade_pool)
            .await
            .expect("insert agent");
    }
    sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_cek_a','env_cek_a','agt_cek_a','legacy_rsa','RSA-OAEP-256',$1)")
        .bind("A".repeat(128))
        .execute(&upgrade_pool)
        .await
        .expect("insert legacy RSA key before algorithm migration");

    PostgresStore::from_pool(upgrade_pool.clone())
        .migrate()
        .await
        .expect("application startup upgrades through algorithm migration");
    let latest: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&upgrade_pool)
        .await
        .expect("read upgraded schema version");
    assert_eq!(latest, 47);
    let reference_guard_config: Vec<String> = sqlx::query_scalar(
        "SELECT coalesce(proconfig, ARRAY[]::text[])
         FROM pg_proc JOIN pg_namespace ON pg_namespace.oid = pg_proc.pronamespace
         WHERE proname = 'guard_encrypted_job_key_reference'
           AND pg_namespace.nspname = current_schema()",
    )
    .fetch_one(&upgrade_pool)
    .await
    .expect("inspect reference guard search path");
    assert_eq!(
        reference_guard_config,
        vec![format!("search_path={upgrade_schema}, pg_catalog, pg_temp")],
        "reference guard must use only its owning schema and trusted system schemas"
    );
    let legacy_algorithm: String = sqlx::query_scalar(
        "SELECT algorithm FROM node_content_encryption_keys
         WHERE workspace_id = 'wsp_cek_a' AND environment_id = 'env_cek_a'
           AND agent_id = 'agt_cek_a' AND key_id = 'legacy_rsa'",
    )
    .fetch_one(&upgrade_pool)
    .await
    .expect("legacy key remains readable after forward migration");
    assert_eq!(legacy_algorithm, "RSA-OAEP-256");

    sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_cek_b','env_cek_b','agt_cek_b','v3_ecdh','ECDH-P256-HKDF-SHA256',$1)")
        .bind("B".repeat(80))
        .execute(&upgrade_pool)
        .await
        .expect("v3 P-256 ECDH key is accepted after migration");
    let lifecycle: String = sqlx::query_scalar(
        "SELECT lifecycle_state FROM node_content_encryption_keys
         WHERE workspace_id = 'wsp_cek_b' AND environment_id = 'env_cek_b'
           AND agent_id = 'agt_cek_b' AND key_id = 'v3_ecdh'",
    )
    .fetch_one(&upgrade_pool)
    .await
    .expect("read migrated lifecycle state");
    assert_eq!(lifecycle, "active");
    sqlx::query(
        "UPDATE node_content_encryption_keys
         SET lifecycle_state = 'decrypt_only', state_changed_at = now()
         WHERE workspace_id = 'wsp_cek_b' AND environment_id = 'env_cek_b'
           AND agent_id = 'agt_cek_b' AND key_id = 'v3_ecdh'",
    )
    .execute(&upgrade_pool)
    .await
    .expect("active key becomes decrypt-only");
    sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_cek_b','env_cek_b','agt_cek_b','v3_next','ECDH-P256-HKDF-SHA256',$1)")
        .bind("N".repeat(80)).execute(&upgrade_pool).await.expect("one replacement active key");
    let mutation = sqlx::query(
        "UPDATE node_content_encryption_keys SET public_key_spki = $1
         WHERE workspace_id = 'wsp_cek_b' AND environment_id = 'env_cek_b'
           AND agent_id = 'agt_cek_b' AND key_id = 'v3_ecdh'",
    )
    .bind("M".repeat(80))
    .execute(&upgrade_pool)
    .await;
    assert!(
        mutation.is_err(),
        "registered key material must be immutable"
    );

    sqlx::query("INSERT INTO printers (id,workspace_id,environment_id,agent_id,native_id,name) VALUES ('prn_cek_b','wsp_cek_b','env_cek_b','agt_cek_b','native','Test')")
        .execute(&upgrade_pool).await.expect("insert lifecycle test printer");
    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,state_sequence,per_printer_sequence,expires_at,created_at,updated_at) VALUES ('job_cek_b','wsp_cek_b','env_cek_b','prn_cek_b','agt_cek_b','{}'::jsonb,'queued',1,1,now()+interval '1 hour',now(),now())")
        .execute(&upgrade_pool).await.expect("insert lifecycle test job");
    sqlx::query("INSERT INTO encrypted_job_key_references (workspace_id,environment_id,agent_id,key_id,job_id) VALUES ('wsp_cek_b','env_cek_b','agt_cek_b','v3_ecdh','job_cek_b')")
        .execute(&upgrade_pool).await.expect("record tenant-scoped job key reference");
    let referenced_revoke = sqlx::query(
        "UPDATE node_content_encryption_keys
         SET lifecycle_state = 'revoked', revoked_at = now(), state_changed_at = now()
         WHERE workspace_id = 'wsp_cek_b' AND environment_id = 'env_cek_b'
           AND agent_id = 'agt_cek_b' AND key_id = 'v3_ecdh'",
    )
    .execute(&upgrade_pool)
    .await;
    assert!(
        referenced_revoke.is_err(),
        "referenced keys cannot be revoked"
    );
    let cross_tenant_reference = sqlx::query("INSERT INTO encrypted_job_key_references (workspace_id,environment_id,agent_id,key_id,job_id) VALUES ('wsp_cek_a','env_cek_a','agt_cek_b','v3_ecdh','job_cek_b')")
        .execute(&upgrade_pool).await;
    assert!(
        cross_tenant_reference.is_err(),
        "key references cannot cross tenant boundaries"
    );

    let mut revoke_first = upgrade_pool.begin().await.expect("begin concurrent revoke");
    sqlx::query(
        "UPDATE node_content_encryption_keys
         SET lifecycle_state = 'revoked', revoked_at = now(), state_changed_at = now()
         WHERE workspace_id = 'wsp_cek_b' AND environment_id = 'env_cek_b'
           AND agent_id = 'agt_cek_b' AND key_id = 'v3_next'",
    )
    .execute(&mut *revoke_first)
    .await
    .expect("stage uncommitted revoke");
    let blocked_pool = upgrade_pool.clone();
    let (pid_send, pid_receive) = tokio::sync::oneshot::channel();
    let blocked_reference = tokio::spawn(async move {
        let mut connection = blocked_pool
            .acquire()
            .await
            .expect("acquire reference connection");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *connection)
            .await
            .expect("reference backend pid");
        let _ = pid_send.send(pid);
        sqlx::query("INSERT INTO encrypted_job_key_references (workspace_id,environment_id,agent_id,key_id,job_id) VALUES ('wsp_cek_b','env_cek_b','agt_cek_b','v3_next','job_cek_b')")
            .execute(&mut *connection).await
    });
    let blocked_pid = pid_receive.await.expect("receive reference backend pid");
    wait_for_database_blocker(&upgrade_pool, blocked_pid).await;
    revoke_first.commit().await.expect("commit revoke first");
    assert!(
        blocked_reference
            .await
            .expect("join blocked reference")
            .is_err(),
        "a revoke that wins the row lock must reject the later reference"
    );

    sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_cek_b','env_cek_b','agt_cek_b','v3_insert_first','ECDH-P256-HKDF-SHA256',$1)")
        .bind("I".repeat(80)).execute(&upgrade_pool).await.expect("insert concurrency key");
    let mut reference_first = upgrade_pool
        .begin()
        .await
        .expect("begin concurrent reference");
    sqlx::query("INSERT INTO encrypted_job_key_references (workspace_id,environment_id,agent_id,key_id,job_id) VALUES ('wsp_cek_b','env_cek_b','agt_cek_b','v3_insert_first','job_cek_b')")
        .execute(&mut *reference_first).await.expect("stage uncommitted reference");
    let blocked_pool = upgrade_pool.clone();
    let (pid_send, pid_receive) = tokio::sync::oneshot::channel();
    let blocked_revoke = tokio::spawn(async move {
        let mut connection = blocked_pool
            .acquire()
            .await
            .expect("acquire revoke connection");
        let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *connection)
            .await
            .expect("revoke backend pid");
        let _ = pid_send.send(pid);
        sqlx::query(
            "UPDATE node_content_encryption_keys
             SET lifecycle_state = 'revoked', revoked_at = now(), state_changed_at = now()
             WHERE workspace_id = 'wsp_cek_b' AND environment_id = 'env_cek_b'
               AND agent_id = 'agt_cek_b' AND key_id = 'v3_insert_first'",
        )
        .execute(&mut *connection)
        .await
    });
    let blocked_pid = pid_receive.await.expect("receive revoke backend pid");
    wait_for_database_blocker(&upgrade_pool, blocked_pid).await;
    reference_first
        .commit()
        .await
        .expect("commit reference first");
    assert!(
        blocked_revoke.await.expect("join blocked revoke").is_err(),
        "a reference that wins the row lock must make revoke fail"
    );
    let retarget_revoked = sqlx::query(
        "UPDATE encrypted_job_key_references SET key_id = 'v3_next'
         WHERE workspace_id = 'wsp_cek_b' AND environment_id = 'env_cek_b'
           AND agent_id = 'agt_cek_b' AND key_id = 'v3_ecdh'
           AND job_id = 'job_cek_b'",
    )
    .execute(&upgrade_pool)
    .await;
    assert!(
        retarget_revoked.is_err(),
        "reference updates cannot target a revoked key"
    );
    let cross_tenant_delete = sqlx::query(
        "DELETE FROM jobs
         WHERE id = 'job_cek_b' AND workspace_id = 'wsp_cek_a'
           AND environment_id = 'env_cek_a'",
    )
    .execute(&upgrade_pool)
    .await
    .expect("cross-tenant job deletion probe");
    assert_eq!(cross_tenant_delete.rows_affected(), 0);
    let exact_delete = sqlx::query(
        "DELETE FROM jobs
         WHERE id = 'job_cek_b' AND workspace_id = 'wsp_cek_b'
           AND environment_id = 'env_cek_b'",
    )
    .execute(&upgrade_pool)
    .await
    .expect("delete exact retained encrypted job");
    assert_eq!(exact_delete.rows_affected(), 1);
    let remaining_references: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM encrypted_job_key_references
         WHERE workspace_id = 'wsp_cek_b' AND environment_id = 'env_cek_b'
           AND job_id = 'job_cek_b'",
    )
    .fetch_one(&upgrade_pool)
    .await
    .expect("count references after exact job deletion");
    assert_eq!(remaining_references, 0);
    sqlx::query(
        "UPDATE node_content_encryption_keys
         SET lifecycle_state = 'revoked', revoked_at = now(), state_changed_at = now()
         WHERE workspace_id = 'wsp_cek_b' AND environment_id = 'env_cek_b'
           AND agent_id = 'agt_cek_b' AND key_id = 'v3_ecdh'",
    )
    .execute(&upgrade_pool)
    .await
    .expect("job deletion releases its key reference transactionally");
    let unsupported = sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_cek_b','env_cek_b','agt_cek_b','unknown_suite','ECDH-P384-HKDF-SHA384',$1)")
        .bind("C".repeat(128))
        .execute(&upgrade_pool)
        .await;
    assert!(
        unsupported.is_err(),
        "unapproved algorithms must remain rejected"
    );
    let cross_tenant = sqlx::query("INSERT INTO node_content_encryption_keys (workspace_id,environment_id,agent_id,key_id,algorithm,public_key_spki) VALUES ('wsp_cek_a','env_cek_a','agt_cek_b','cross_tenant','ECDH-P256-HKDF-SHA256',$1)")
        .bind("D".repeat(80))
        .execute(&upgrade_pool)
        .await;
    assert!(
        cross_tenant.is_err(),
        "composite agent foreign key must reject cross-tenant key registration"
    );

    upgrade_pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {upgrade_schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact upgrade schema");
}
