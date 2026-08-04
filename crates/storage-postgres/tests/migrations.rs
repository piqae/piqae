#![allow(clippy::expect_used)]

use piqae_storage_postgres::PostgresStore;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::{borrow::Cow, env};

async fn schema_pool(database_url: &str, schema: &str) -> PgPool {
    let schema = schema.to_owned();
    PgPoolOptions::new()
        .max_connections(2)
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
    let expected_latest = all
        .iter()
        .map(|migration| migration.version)
        .max()
        .expect("at least one PostgreSQL migration");
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
    assert_eq!(latest, expected_latest);
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
