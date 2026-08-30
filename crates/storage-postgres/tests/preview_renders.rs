#![allow(clippy::expect_used)]

use piqae_domain::{EnvironmentId, WorkspaceId};
use piqae_storage_postgres::PostgresStore;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{borrow::Cow, env};

async fn schema_pool(database_url: &str, schema: &str) -> PgPool {
    let schema = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(4)
        .after_connect(move |connection, _| {
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

async fn seed_tenant(pool: &PgPool, workspace: WorkspaceId, environment: EnvironmentId) {
    sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,'Preview','preview')")
        .bind(workspace.to_string())
        .execute(pool)
        .await
        .expect("workspace fixture");
    sqlx::query(
        "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'test','Test')",
    )
    .bind(environment.to_string())
    .bind(workspace.to_string())
    .execute(pool)
    .await
    .expect("environment fixture");
}

async fn insert_printable_fixture(
    pool: &PgPool,
    workspace: WorkspaceId,
    environment: EnvironmentId,
) {
    let ciphertext = vec![7_u8; 32];
    sqlx::query(
        "INSERT INTO document_templates
         (id,workspace_id,environment_id,name,draft_ciphertext,draft_sha256)
         VALUES ('dtpl_preview_fixture',$1,$2,'Fixture',$3,$4)",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(&ciphertext)
    .bind("a".repeat(64))
    .execute(pool)
    .await
    .expect("template fixture");
    sqlx::query(
        "INSERT INTO document_template_revisions
         (id,workspace_id,environment_id,template_id,revision,spec_ciphertext,spec_sha256,renderer_profile)
         VALUES ('drev_preview_fixture',$1,$2,'dtpl_preview_fixture',1,$3,$4,'printpacket/v1')",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(&ciphertext)
    .bind("b".repeat(64))
    .execute(pool)
    .await
    .expect("revision fixture");
    sqlx::query(
        "INSERT INTO document_renders
         (id,workspace_id,environment_id,template_revision_id,input_ciphertext,input_sha256,
          idempotency_key,request_sha256)
         VALUES ('drnd_printable_fixture',$1,$2,'drev_preview_fixture',$3,$4,
                 'printable-fixture-key',$5)",
    )
    .bind(workspace.to_string())
    .bind(environment.to_string())
    .bind(&ciphertext)
    .bind("c".repeat(64))
    .bind("d".repeat(64))
    .execute(pool)
    .await
    .expect("printable render fixture");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn preview_render_migrates_fresh_and_n_minus_one_with_hard_purpose_fences() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for preview render migration evidence");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    let all = sqlx::migrate!("../../migrations/postgres");

    for mode in ["upgrade", "fresh"] {
        let schema = format!("piqae_preview_{mode}_{}", ulid::Ulid::new()).to_ascii_lowercase();
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .expect("create exact disposable schema");
        let pool = schema_pool(&database_url, &schema).await;
        let workspace = WorkspaceId::new();
        let environment = EnvironmentId::new();
        if mode == "upgrade" {
            let previous = Migrator {
                migrations: Cow::Owned(
                    all.iter()
                        .filter(|migration| migration.version < 46)
                        .cloned()
                        .collect(),
                ),
                ignore_missing: false,
                locking: true,
                no_tx: false,
            };
            previous.run(&pool).await.expect("apply exact 0045 schema");
            seed_tenant(&pool, workspace, environment).await;
            insert_printable_fixture(&pool, workspace, environment).await;
        }
        PostgresStore::from_pool(pool.clone())
            .migrate()
            .await
            .expect("migrate through preview renders");
        let latest: i64 =
            sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
                .fetch_one(&pool)
                .await
                .expect("latest migration");
        assert_eq!(latest, 46);
        if mode == "fresh" {
            seed_tenant(&pool, workspace, environment).await;
            insert_printable_fixture(&pool, workspace, environment).await;
        }
        let printable_purpose: String = sqlx::query_scalar(
            "SELECT purpose FROM document_renders WHERE id='drnd_printable_fixture'",
        )
        .fetch_one(&pool)
        .await
        .expect("backfilled printable purpose");
        assert_eq!(printable_purpose, "printable");

        let ciphertext = vec![9_u8; 32];
        sqlx::query(
            "INSERT INTO document_renders
             (id,workspace_id,environment_id,purpose,spec_ciphertext,spec_sha256,
              input_ciphertext,input_sha256,idempotency_key,request_sha256,state,
              artifact_object_key_ciphertext,artifact_sha256,artifact_byte_length,
              artifact_media_type,page_count,expires_at)
             VALUES ('dprv_live_fixture',$1,$2,'preview',$3,$4,$5,$6,
                     'preview-live-key',$7,'completed',$8,$9,8,'application/pdf',1,
                     now()+interval '15 minutes')",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind(&ciphertext)
        .bind("1".repeat(64))
        .bind(&ciphertext)
        .bind("2".repeat(64))
        .bind("3".repeat(64))
        .bind(&ciphertext)
        .bind("4".repeat(64))
        .execute(&pool)
        .await
        .expect("live preview fixture");

        let cross_tenant_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM document_renders
             WHERE id='dprv_live_fixture' AND workspace_id=$1 AND environment_id=$2",
        )
        .bind(WorkspaceId::new().to_string())
        .bind(EnvironmentId::new().to_string())
        .fetch_one(&pool)
        .await
        .expect("cross-tenant probe");
        assert_eq!(cross_tenant_count, 0);

        let approval = sqlx::query(
            "INSERT INTO document_previews
             (id,workspace_id,environment_id,render_id,idempotency_key,request_sha256,expires_at)
             VALUES ('dpvw_forbidden',$1,$2,'dprv_live_fixture','preview-approval-key',$3,
                     now()+interval '5 minutes')",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind("5".repeat(64))
        .execute(&pool)
        .await;
        assert!(approval.is_err(), "preview render cannot enter approval");

        let upload = sqlx::query(
            "INSERT INTO uploads
             (id,workspace_id,environment_id,object_key,media_type,expected_sha256,
              expected_bytes,state,expires_at,completed_at,source_document_render_id,acquisition_sha256)
             VALUES ('upl_forbidden',$1,$2,'forbidden-preview.pdf','application/pdf',$3,
                     8,'complete',now()+interval '5 minutes',now(),'dprv_live_fixture',$4)",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind("4".repeat(64))
        .bind("6".repeat(64))
        .execute(&pool)
        .await;
        assert!(upload.is_err(), "preview render cannot back a print upload");

        // Exercise the independent jobs trigger with a deliberately malformed
        // row that simulates data written before the upload guard existed.
        // The trigger must inspect the source render rather than fail open.
        sqlx::query(
            "ALTER TABLE uploads DISABLE TRIGGER uploads_guard_printable_document_artifact",
        )
        .execute(&pool)
        .await
        .expect("temporarily disable upload guard for legacy-row fixture");
        sqlx::query(
            "INSERT INTO uploads
             (id,workspace_id,environment_id,object_key,media_type,expected_sha256,
              expected_bytes,state,expires_at,completed_at,source_document_render_id,acquisition_sha256)
             VALUES ('upl_legacy_preview',$1,$2,'legacy-preview.pdf','application/pdf',$3,
                     8,'complete',now()+interval '5 minutes',now(),'dprv_live_fixture',$4)",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind("4".repeat(64))
        .bind("6".repeat(64))
        .execute(&pool)
        .await
        .expect("legacy malformed preview upload fixture");
        sqlx::query("ALTER TABLE uploads ENABLE TRIGGER uploads_guard_printable_document_artifact")
            .execute(&pool)
            .await
            .expect("restore upload purpose guard");
        sqlx::query(
            "INSERT INTO agents
             (id,workspace_id,environment_id,name,installation_id,os,architecture,version,protocol_version)
             VALUES ('agt_preview_guard',$1,$2,'Preview guard','install-preview-guard',
                     'linux','x86_64','test',1)",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .execute(&pool)
        .await
        .expect("preview guard agent fixture");
        sqlx::query(
            "INSERT INTO printers
             (id,workspace_id,environment_id,agent_id,native_id,name)
             VALUES ('prn_preview_guard',$1,$2,'agt_preview_guard','virtual-preview-guard','Virtual')",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .execute(&pool)
        .await
        .expect("preview guard printer fixture");
        let job = sqlx::query(
            "INSERT INTO jobs
             (id,workspace_id,environment_id,printer_id,agent_id,payload,state,
              per_printer_sequence,expires_at)
             VALUES ('job_forbidden_preview',$1,$2,'prn_preview_guard','agt_preview_guard',
              '{\"content\":{\"type\":\"upload\",\"upload_id\":\"upl_legacy_preview\"}}'::jsonb,
              'registered',1,now()+interval '5 minutes')",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .execute(&pool)
        .await;
        assert!(job.is_err(), "preview render cannot back a print job");

        sqlx::query(
            "INSERT INTO document_renders
             (id,workspace_id,environment_id,purpose,spec_ciphertext,spec_sha256,
              input_ciphertext,input_sha256,idempotency_key,request_sha256,created_at,expires_at)
             VALUES ('dprv_expired_fixture',$1,$2,'preview',$3,$4,$5,$6,
                     'preview-expired-key',$7,now()-interval '2 minutes',now()-interval '1 minute')",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .bind(&ciphertext)
        .bind("7".repeat(64))
        .bind(&ciphertext)
        .bind("8".repeat(64))
        .bind("9".repeat(64))
        .execute(&pool)
        .await
        .expect("expired preview fixture");
        let store = PostgresStore::from_pool(pool.clone());
        let claims = store
            .claim_document_renders("preview-test", 100, 30)
            .await
            .expect("claim renders");
        assert!(
            claims
                .iter()
                .all(|work| work.render.id != "dprv_expired_fixture")
        );
        let expiry = store
            .claim_expired_document_artifacts("preview-test", 100, 30)
            .await
            .expect("claim expired preview")
            .into_iter()
            .find(|work| work.render_id == "dprv_expired_fixture")
            .expect("expired preview cleanup work");
        store
            .complete_document_artifact_expiry(&expiry)
            .await
            .expect("clear expired preview source");
        let cleared: (String, Option<Vec<u8>>, Option<Vec<u8>>) = sqlx::query_as(
            "SELECT state,input_ciphertext,spec_ciphertext FROM document_renders
             WHERE id='dprv_expired_fixture' AND workspace_id=$1 AND environment_id=$2",
        )
        .bind(workspace.to_string())
        .bind(environment.to_string())
        .fetch_one(&pool)
        .await
        .expect("inspect cleared preview");
        assert_eq!(cleared.0, "expired");
        assert!(cleared.1.is_none() && cleared.2.is_none());

        pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .expect("drop exact disposable schema");
    }
    admin.close().await;
}
