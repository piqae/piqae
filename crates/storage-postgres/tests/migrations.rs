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

#[tokio::test]
async fn business_document_cutover_resets_only_prerelease_document_rows() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run PostgreSQL migration evidence");
        return;
    };
    let schema = format!("piqae_business_cutover_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
                .filter(|migration| migration.version < 38)
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
        .expect("apply prerelease document schema");
    sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ('wsp_cutover','Cutover','cutover')")
        .execute(&pool)
        .await
        .expect("workspace fixture");
    sqlx::query("INSERT INTO environments (id,workspace_id,kind,name) VALUES ('env_cutover','wsp_cutover','live','Live')")
        .execute(&pool).await.expect("environment fixture");
    sqlx::query("INSERT INTO agents (id,workspace_id,environment_id,name,installation_id,os,architecture,version,protocol_version) VALUES ('agt_cutover','wsp_cutover','env_cutover','Node','install-cutover','linux','x86_64','test',1)")
        .execute(&pool).await.expect("agent fixture");
    sqlx::query("INSERT INTO printers (id,workspace_id,environment_id,agent_id,native_id,name) VALUES ('prn_cutover','wsp_cutover','env_cutover','agt_cutover','virtual','Virtual')")
        .execute(&pool).await.expect("printer fixture");
    sqlx::query("INSERT INTO jobs (id,workspace_id,environment_id,printer_id,agent_id,payload,state,per_printer_sequence,expires_at) VALUES ('job_cutover','wsp_cutover','env_cutover','prn_cutover','agt_cutover','{}'::jsonb,'registered',1,now()+interval '1 hour')")
        .execute(&pool).await.expect("unrelated job fixture");
    let ciphertext = vec![7_u8; 29];
    sqlx::query("INSERT INTO document_templates (id,workspace_id,environment_id,name,draft_ciphertext,draft_sha256) VALUES ('dtpl_cutover','wsp_cutover','env_cutover','Old',$1,$2)")
        .bind(&ciphertext).bind("a".repeat(64)).execute(&pool).await.expect("old template fixture");
    sqlx::query("INSERT INTO document_conversions (id,workspace_id,environment_id,adapter_id,adapter_version,adapter_api_version,source_format,source_sha256,strict,fidelity,renderer_version,result_ciphertext,result_sha256,idempotency_key,request_sha256) VALUES ('dcnv_cutover','wsp_cutover','env_cutover','pdfme','1.0.0','piqae.adapter/v1','pdfme.template',$1,true,'exact','old',$2,$1,'conversion-cutover',$1)")
        .bind("a".repeat(64)).bind(&ciphertext).execute(&pool).await.expect("old conversion fixture");
    PostgresStore::from_pool(pool.clone())
        .migrate()
        .await
        .expect("apply hard cutover");
    let documents: i64 = sqlx::query_scalar("SELECT count(*) FROM document_templates")
        .fetch_one(&pool)
        .await
        .expect("count reset templates");
    let unrelated_jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE id='job_cutover' AND workspace_id='wsp_cutover' AND environment_id='env_cutover'")
        .fetch_one(&pool).await.expect("count retained job");
    let account: i64 = sqlx::query_scalar("SELECT count(*) FROM workspaces WHERE id='wsp_cutover'")
        .fetch_one(&pool)
        .await
        .expect("count retained workspace");
    let conversion_table_removed: bool =
        sqlx::query_scalar("SELECT to_regclass('document_conversions') IS NULL")
            .fetch_one(&pool)
            .await
            .expect("inspect removed conversion table");
    assert_eq!(documents, 0);
    assert_eq!(unrelated_jobs, 1);
    assert_eq!(account, 1);
    assert!(conversion_table_removed);
    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop exact disposable schema");
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
    assert_eq!(latest, 41);
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
        VALUES ('drev_cross','wsp_doc_b','env_doc_b','dtpl_doc_a',1,$1,$2,'piqae.business-document/v1')")
        .bind(ciphertext.as_slice()).bind("a".repeat(64)).execute(&pool).await;
    assert!(
        cross_tenant_revision.is_err(),
        "composite tenant foreign key must reject probing"
    );
    sqlx::query(
        "INSERT INTO document_template_revisions
         (id,workspace_id,environment_id,template_id,revision,spec_ciphertext,spec_sha256,renderer_profile)
         VALUES ('drev_doc_a','wsp_doc_a','env_doc_a','dtpl_doc_a',1,$1,$2,'piqae.business-document/v1')",
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
    let hosted_conversion_removed: bool =
        sqlx::query_scalar("SELECT to_regclass('document_conversions') IS NULL")
            .fetch_one(&pool)
            .await
            .expect("inspect hosted conversion removal");
    assert!(hosted_conversion_removed);
    let latest: i64 = sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .expect("read schema version");
    assert_eq!(latest, 41);
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
    assert_eq!(latest, 41);
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
    assert_eq!(empty_latest, 41);
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
    assert_eq!(latest, 41);
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
