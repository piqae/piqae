#![allow(clippy::expect_used)]

use piqae_storage_postgres::PostgresStore;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{borrow::Cow, env};

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

async fn seed_tenant(pool: &PgPool, suffix: &str, blocked: bool) {
    sqlx::query("INSERT INTO workspaces (id,name,slug) VALUES ($1,$2,$3)")
        .bind(format!("wsp_recovery_{suffix}"))
        .bind(format!("Recovery {suffix}"))
        .bind(format!("recovery-{suffix}"))
        .execute(pool)
        .await
        .expect("workspace fixture");
    sqlx::query(
        "INSERT INTO environments (id,workspace_id,kind,name) VALUES ($1,$2,'test','Test')",
    )
    .bind(format!("env_recovery_{suffix}"))
    .bind(format!("wsp_recovery_{suffix}"))
    .execute(pool)
    .await
    .expect("environment fixture");
    sqlx::query(
        "INSERT INTO agents (
             id,workspace_id,environment_id,name,installation_id,
             os,architecture,version,protocol_version
         ) VALUES ($1,$2,$3,'Node',$4,'test','test','1',1)",
    )
    .bind(format!("agt_recovery_{suffix}"))
    .bind(format!("wsp_recovery_{suffix}"))
    .bind(format!("env_recovery_{suffix}"))
    .bind(format!("installation-recovery-{suffix}"))
    .execute(pool)
    .await
    .expect("agent fixture");
    sqlx::query(
        "INSERT INTO printers (
             id,workspace_id,environment_id,agent_id,native_id,name
         ) VALUES ($1,$2,$3,$4,$5,'Virtual printer')",
    )
    .bind(format!("prt_recovery_{suffix}"))
    .bind(format!("wsp_recovery_{suffix}"))
    .bind(format!("env_recovery_{suffix}"))
    .bind(format!("agt_recovery_{suffix}"))
    .bind(format!("native-recovery-{suffix}"))
    .execute(pool)
    .await
    .expect("printer fixture");
    let state = if blocked {
        "blocked"
    } else {
        "waiting_for_agent"
    };
    sqlx::query(
        "INSERT INTO jobs (
             id,workspace_id,environment_id,printer_id,agent_id,payload,state,
             state_sequence,per_printer_sequence,expires_at
         ) VALUES ($1,$2,$3,$4,$5,'{}',$6,1,1,now()+interval '1 hour')",
    )
    .bind(format!("job_recovery_{suffix}"))
    .bind(format!("wsp_recovery_{suffix}"))
    .bind(format!("env_recovery_{suffix}"))
    .bind(format!("prt_recovery_{suffix}"))
    .bind(format!("agt_recovery_{suffix}"))
    .bind(state)
    .execute(pool)
    .await
    .expect("job fixture");
    if blocked {
        sqlx::query(
            "INSERT INTO job_events (
                 id,workspace_id,environment_id,job_id,sequence,state,payload,occurred_at
             ) VALUES ($1,$2,$3,$4,1,'blocked',$5,now())",
        )
        .bind(format!("evt_recovery_{suffix}"))
        .bind(format!("wsp_recovery_{suffix}"))
        .bind(format!("env_recovery_{suffix}"))
        .bind(format!("job_recovery_{suffix}"))
        .bind(serde_json::json!({
            "reason": "node_update_required",
            "agent_id": null
        }))
        .execute(pool)
        .await
        .expect("capability block event fixture");
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn capability_recovery_checks_migrate_fresh_and_0044_with_tenant_fences() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL for recovery migration evidence");
        return;
    };
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL test database");
    let all = sqlx::migrate!("../../migrations/postgres");

    for mode in ["upgrade", "fresh"] {
        let schema =
            format!("piqae_capability_recovery_{mode}_{}", ulid::Ulid::new()).to_ascii_lowercase();
        sqlx::query(&format!("CREATE SCHEMA {schema}"))
            .execute(&admin)
            .await
            .expect("create exact disposable schema");
        let pool = schema_pool(&database_url, &schema).await;
        if mode == "upgrade" {
            let previous = Migrator {
                migrations: Cow::Owned(
                    all.iter()
                        .filter(|migration| migration.version < 45)
                        .cloned()
                        .collect(),
                ),
                ignore_missing: false,
                locking: true,
                no_tx: false,
            };
            previous.run(&pool).await.expect("apply exact 0044 schema");
            seed_tenant(&pool, "a", true).await;
            assert!(
                sqlx::query("SELECT state FROM jobs WHERE id='job_recovery_a'")
                    .fetch_one(&pool)
                    .await
                    .is_ok(),
                "the 0044 application can continue reading its job projection before 0045"
            );
        }
        PostgresStore::from_pool(pool.clone())
            .migrate()
            .await
            .expect("migrate through version 46");
        let latest: i64 =
            sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
                .fetch_one(&pool)
                .await
                .expect("latest migration");
        assert_eq!(latest, 46);

        if mode == "fresh" {
            seed_tenant(&pool, "a", false).await;
        } else {
            let inferred: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM node_capability_recovery_checks
                 WHERE workspace_id='wsp_recovery_a'
                   AND environment_id='env_recovery_a'
                   AND agent_id='agt_recovery_a'
                   AND job_id='job_recovery_a'",
            )
            .fetch_one(&pool)
            .await
            .expect("inspect unreleased intermediate block");
            assert_eq!(
                inferred, 0,
                "0045 must not infer recoverability for a block lacking its canonical row"
            );
        }
        seed_tenant(&pool, "b", false).await;
        sqlx::query(
            "INSERT INTO node_capability_recovery_checks (
                 workspace_id,environment_id,agent_id,job_id
             ) VALUES ('wsp_recovery_a','env_recovery_a','agt_recovery_a','job_recovery_a')
             ON CONFLICT DO NOTHING",
        )
        .execute(&pool)
        .await
        .expect("valid tenant-scoped recovery check");
        assert!(
            sqlx::query(
                "INSERT INTO node_capability_recovery_checks (
                     workspace_id,environment_id,agent_id,job_id
                 ) VALUES ('wsp_recovery_b','env_recovery_b','agt_recovery_b','job_recovery_a')",
            )
            .execute(&pool)
            .await
            .is_err(),
            "a cross-tenant job reference must fail"
        );
        assert!(
            sqlx::query(
                "INSERT INTO node_capability_recovery_checks (
                     workspace_id,environment_id,agent_id,job_id
                 ) VALUES ('wsp_recovery_a','env_recovery_a','agt_recovery_b','job_recovery_a')",
            )
            .execute(&pool)
            .await
            .is_err(),
            "a cross-tenant agent reference must fail"
        );
        assert!(
            sqlx::query(
                "UPDATE node_capability_recovery_checks
                 SET checked_at=now(),next_check_at=now()-interval '1 second'
                 WHERE workspace_id='wsp_recovery_a'
                   AND environment_id='env_recovery_a'
                   AND agent_id='agt_recovery_a'
                   AND job_id='job_recovery_a'",
            )
            .execute(&pool)
            .await
            .is_err(),
            "next-check progress cannot move behind its completed check"
        );
        sqlx::query("DELETE FROM jobs WHERE id='job_recovery_a'")
            .execute(&pool)
            .await
            .expect("delete exact fixture job");
        let cascaded: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM node_capability_recovery_checks
             WHERE job_id='job_recovery_a'",
        )
        .fetch_one(&pool)
        .await
        .expect("inspect recovery cascade");
        assert_eq!(cascaded, 0);

        pool.close().await;
        sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
            .execute(&admin)
            .await
            .expect("drop exact disposable schema");
    }
}
