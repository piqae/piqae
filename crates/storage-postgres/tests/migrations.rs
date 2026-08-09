#![allow(clippy::expect_used)]

use piqae_storage_postgres::PostgresStore;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{borrow::Cow, env};

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
    assert_eq!(latest, 31);
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
    assert_eq!(latest, 30);
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
    assert_eq!(empty_latest, 30);
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
    assert_eq!(latest, 30);
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
