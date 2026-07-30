#![allow(clippy::expect_used, clippy::too_many_lines)]

use chrono::{Duration, Utc};
use piqae_storage_postgres::{
    PostgresStore, StorageError, WorkOsIdentityData, WorkOsIdentityEvent, WorkOsMembershipAccess,
    WorkOsProjectionResult,
};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::env;

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

fn membership_event(
    id: &str,
    hash_byte: char,
    organization_id: &str,
    role: &str,
    status: &str,
    event_at: chrono::DateTime<Utc>,
) -> WorkOsIdentityEvent {
    WorkOsIdentityEvent {
        id: id.into(),
        event_type: "organization_membership.updated".into(),
        payload_sha256: hash_byte.to_string().repeat(64),
        data: WorkOsIdentityData::Membership {
            membership_id: format!("om_{organization_id}"),
            organization_id: organization_id.into(),
            user_id: "user_shared".into(),
            role: role.into(),
            status: status.into(),
            event_at,
        },
    }
}

#[tokio::test]
async fn workos_projection_is_idempotent_ordered_and_organization_scoped() {
    let Some(database_url) = env::var("PIQAE_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: set PIQAE_TEST_DATABASE_URL to run WorkOS projection evidence");
        return;
    };
    let schema = format!("piqae_workos_test_{}", ulid::Ulid::new()).to_ascii_lowercase();
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
    store.migrate().await.expect("apply migrations");

    let now = Utc::now() - Duration::minutes(1);
    let active = membership_event("event_active", 'a', "org_alpha", "operator", "active", now);
    assert_eq!(
        store
            .project_workos_identity_event(&active)
            .await
            .expect("project active membership"),
        WorkOsProjectionResult::Applied
    );
    assert_eq!(
        store
            .project_workos_identity_event(&active)
            .await
            .expect("replay exact event"),
        WorkOsProjectionResult::Duplicate
    );
    assert_eq!(
        store
            .workos_membership_access("org_alpha", "user_shared")
            .await
            .expect("active membership lookup"),
        WorkOsMembershipAccess::Active
    );

    let stale = membership_event(
        "event_stale",
        'b',
        "org_alpha",
        "owner",
        "active",
        now - Duration::minutes(1),
    );
    assert_eq!(
        store
            .project_workos_identity_event(&stale)
            .await
            .expect("accept stale event without applying"),
        WorkOsProjectionResult::Stale
    );
    let role: String = sqlx::query_scalar(
        "SELECT member.role
         FROM workspace_members member
         JOIN workspaces workspace ON workspace.id = member.workspace_id
         WHERE workspace.identity_organization_id = 'org_alpha'",
    )
    .fetch_one(&pool)
    .await
    .expect("read projected role");
    assert_eq!(role, "operator");

    let conflicting = WorkOsIdentityEvent {
        payload_sha256: "c".repeat(64),
        ..active.clone()
    };
    assert!(matches!(
        store.project_workos_identity_event(&conflicting).await,
        Err(StorageError::IdempotencyConflict)
    ));

    let second = membership_event("event_second_org", 'd', "org_beta", "viewer", "active", now);
    store
        .project_workos_identity_event(&second)
        .await
        .expect("project second organization");
    assert_eq!(
        store
            .workos_membership_access("org_beta", "user_shared")
            .await
            .expect("second organization lookup"),
        WorkOsMembershipAccess::Active
    );
    let rows = sqlx::query(
        "SELECT workspace.identity_organization_id, member.role
         FROM workspace_members member
         JOIN workspaces workspace ON workspace.id = member.workspace_id
         WHERE member.user_id = (
           SELECT id FROM users WHERE workos_user_id = 'user_shared'
         )
         ORDER BY workspace.identity_organization_id",
    )
    .fetch_all(&pool)
    .await
    .expect("read exact organization projections");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].try_get::<String, _>("role").expect("alpha role"),
        "operator"
    );
    assert_eq!(
        rows[1].try_get::<String, _>("role").expect("beta role"),
        "viewer"
    );

    let removed = membership_event(
        "event_removed",
        'e',
        "org_alpha",
        "operator",
        "inactive",
        now + Duration::seconds(1),
    );
    store
        .project_workos_identity_event(&removed)
        .await
        .expect("project membership removal");
    assert_eq!(
        store
            .workos_membership_access("org_alpha", "user_shared")
            .await
            .expect("removed membership lookup"),
        WorkOsMembershipAccess::Denied
    );
    assert_eq!(
        store
            .workos_membership_access("org_beta", "user_shared")
            .await
            .expect("other organization remains active"),
        WorkOsMembershipAccess::Active
    );

    pool.close().await;
    sqlx::query(&format!("DROP SCHEMA {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop disposable schema");
}
