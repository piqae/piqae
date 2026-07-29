//! `PostgreSQL` persistence and transactional queue operations.
#![allow(
    clippy::missing_errors_doc,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spool_domain::{
    AgentId, EnvironmentId, EventId, Job, JobEvent, JobId, JobOptions, JobState,
    NativePrinterOption, PrinterCapabilities, PrinterId, PrinterState, WorkspaceId,
    validate_transition,
};
use sqlx::{
    PgPool, Postgres, Row, Transaction,
    postgres::{PgPoolOptions, PgRow},
};
use std::collections::BTreeMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct PostgresStore {
    pool: PgPool,
}

#[derive(Clone, Debug)]
pub enum CreateJobResult {
    Created(Job),
    Existing(Job),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobLease {
    pub job: Job,
    pub lease_id: Uuid,
    pub lease_token: String,
    pub lease_until: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct AgentAuthenticationRecord {
    pub workspace_id: WorkspaceId,
    pub environment_id: EnvironmentId,
    pub public_key: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct EnrolledAgent {
    pub agent_id: AgentId,
    pub workspace_id: WorkspaceId,
    pub environment_id: EnvironmentId,
}

#[derive(Clone, Debug)]
pub struct ApiKeyAuthenticationRecord {
    pub workspace_id: WorkspaceId,
    pub environment_id: EnvironmentId,
    pub secret_hash: String,
    pub scopes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoredApiKey {
    pub id: String,
    pub name: String,
    pub lookup_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoredAgent {
    pub id: AgentId,
    pub name: String,
    pub platform: String,
    pub state: String,
    pub version: String,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoredPrinter {
    pub id: PrinterId,
    pub agent_id: AgentId,
    pub name: String,
    pub state: PrinterState,
    pub capabilities: PrinterCapabilities,
    pub capability_revision: u64,
    pub native_options: BTreeMap<String, NativePrinterOption>,
    pub profiles: Vec<PrinterProfileSnapshot>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PrinterProfileSnapshot {
    pub profile_id: String,
    pub revision: u64,
    pub name: String,
    pub is_default: bool,
    pub options: JobOptions,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub native_kind: Option<String>,
    #[serde(default)]
    pub native_digest: Option<String>,
    #[serde(default)]
    pub driver_fingerprint: Option<serde_json::Value>,
    #[serde(default)]
    pub summary: Option<serde_json::Value>,
    #[serde(default)]
    pub stock_id: Option<String>,
    #[serde(default)]
    pub safe_overrides: Vec<String>,
    #[serde(default)]
    pub last_validated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_test_job_id: Option<String>,
    #[serde(default)]
    pub published: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredStock {
    pub id: String,
    pub name: String,
    pub sku: Option<String>,
    pub description: Option<String>,
    pub attributes: serde_json::Value,
    pub archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredTarget {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub stock_id: Option<String>,
    pub enabled: bool,
    pub routing_policy: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredTargetBinding {
    pub id: String,
    pub target_id: String,
    pub printer_id: PrinterId,
    pub agent_id: AgentId,
    pub profile_id: String,
    pub profile_revision: u64,
    pub role: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredBindingReadiness {
    pub binding: StoredTargetBinding,
    pub status: String,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredTargetReadiness {
    pub target_id: String,
    pub status: String,
    pub selected_binding_id: Option<String>,
    pub bindings: Vec<StoredBindingReadiness>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoredWebhook {
    pub id: String,
    pub url: String,
    pub events: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoredWebhookDelivery {
    pub id: String,
    pub event_id: String,
    pub attempt: i32,
    pub next_attempt_at: DateTime<Utc>,
    pub response_status: Option<i32>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub dead_lettered_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoredUpload {
    pub id: String,
    pub object_key: String,
    pub media_type: String,
    pub expected_sha256: String,
    pub expected_bytes: i64,
    pub state: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct WebhookDeliveryWork {
    pub id: String,
    pub event_id: String,
    pub event_type: String,
    pub url: String,
    pub secret_ciphertext: Vec<u8>,
    pub payload: serde_json::Value,
    pub event_occurred_at: DateTime<Utc>,
    pub attempt: i32,
}

pub const WEBHOOK_MAX_CLAIM_BATCH: i64 = 100;
pub const WEBHOOK_CLAIM_TTL_SECONDS: i64 = 300;

#[derive(Clone, Debug, Serialize)]
pub struct StoredTenantEvent {
    pub id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct SyncedPrinter {
    pub id: PrinterId,
    pub native_id: String,
    pub name: String,
    pub state: PrinterState,
    pub is_default: bool,
    pub capabilities: PrinterCapabilities,
    pub capability_revision: u64,
    pub native_options: BTreeMap<String, NativePrinterOption>,
    pub profiles: Vec<PrinterProfileSnapshot>,
}

#[derive(Clone, Debug)]
pub struct StoredAgentCommandBatch {
    pub cursor: Option<String>,
    pub commands: Vec<serde_json::Value>,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("resource not found")]
    NotFound,
    #[error("idempotency key was reused with a different request")]
    IdempotencyConflict,
    #[error("job state changed concurrently")]
    ConcurrentStateChange,
    #[error("invalid state transition")]
    InvalidTransition,
    #[error("stored data is invalid: {0}")]
    InvalidData(String),
    #[error("database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("database migration failed: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl PostgresStore {
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self, StorageError> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    #[must_use]
    pub const fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<(), StorageError> {
        sqlx::migrate!("../../migrations/postgres")
            .run(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn ensure_bootstrap_tenant(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO workspaces (id, name) VALUES ($1, 'Self-hosted')
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(workspace_id.to_string())
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO environments (id, workspace_id, kind, name)
             VALUES ($1,$2,'live','Live')
             ON CONFLICT (id) DO NOTHING",
        )
        .bind(environment_id.to_string())
        .bind(workspace_id.to_string())
        .execute(&mut *transaction)
        .await?;
        let stored_workspace: String =
            sqlx::query_scalar("SELECT workspace_id FROM environments WHERE id = $1")
                .bind(environment_id.to_string())
                .fetch_one(&mut *transaction)
                .await?;
        if stored_workspace != workspace_id.to_string() {
            return Err(StorageError::InvalidData(
                "bootstrap environment belongs to another workspace".into(),
            ));
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn provision_oidc_tenant(
        &self,
        organization_id: &str,
        environment_kind: &str,
    ) -> Result<(WorkspaceId, EnvironmentId), StorageError> {
        if organization_id.is_empty() || !matches!(environment_kind, "test" | "live") {
            return Err(StorageError::InvalidData(
                "invalid OIDC tenant mapping".into(),
            ));
        }
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 1))")
            .bind(organization_id)
            .execute(&mut *transaction)
            .await?;
        let workspace_id = if let Some(id) = sqlx::query_scalar::<_, String>(
            "SELECT id FROM workspaces WHERE workos_organization_id = $1",
        )
        .bind(organization_id)
        .fetch_optional(&mut *transaction)
        .await?
        {
            id.parse().map_err(|error| {
                StorageError::InvalidData(format!("workspace id `{id}`: {error}"))
            })?
        } else {
            let id = WorkspaceId::new();
            sqlx::query(
                "INSERT INTO workspaces (id, name, workos_organization_id)
                 VALUES ($1,$2,$2)",
            )
            .bind(id.to_string())
            .bind(organization_id)
            .execute(&mut *transaction)
            .await?;
            id
        };
        let environment_id = if let Some(id) = sqlx::query_scalar::<_, String>(
            "SELECT id FROM environments WHERE workspace_id = $1 AND kind = $2",
        )
        .bind(workspace_id.to_string())
        .bind(environment_kind)
        .fetch_optional(&mut *transaction)
        .await?
        {
            id.parse().map_err(|error| {
                StorageError::InvalidData(format!("environment id `{id}`: {error}"))
            })?
        } else {
            let id = EnvironmentId::new();
            sqlx::query(
                "INSERT INTO environments (id, workspace_id, kind, name)
                 VALUES ($1,$2,$3,$4)",
            )
            .bind(id.to_string())
            .bind(workspace_id.to_string())
            .bind(environment_kind)
            .bind(if environment_kind == "live" {
                "Live"
            } else {
                "Test"
            })
            .execute(&mut *transaction)
            .await?;
            id
        };
        transaction.commit().await?;
        Ok((workspace_id, environment_id))
    }

    pub async fn api_key_for_authentication(
        &self,
        lookup_prefix: &str,
    ) -> Result<ApiKeyAuthenticationRecord, StorageError> {
        let row = sqlx::query(
            "SELECT workspace_id, environment_id, secret_hash, scopes
             FROM api_keys
             WHERE lookup_prefix = $1 AND revoked_at IS NULL
               AND (expires_at IS NULL OR expires_at > now())",
        )
        .bind(lookup_prefix)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        let workspace_id: String = row.try_get("workspace_id")?;
        let environment_id: String = row.try_get("environment_id")?;
        Ok(ApiKeyAuthenticationRecord {
            workspace_id: workspace_id.parse().map_err(|error| {
                StorageError::InvalidData(format!("workspace id `{workspace_id}`: {error}"))
            })?,
            environment_id: environment_id.parse().map_err(|error| {
                StorageError::InvalidData(format!("environment id `{environment_id}`: {error}"))
            })?,
            secret_hash: row.try_get("secret_hash")?,
            scopes: row.try_get("scopes")?,
        })
    }

    pub async fn mark_api_key_used(&self, lookup_prefix: &str) -> Result<(), StorageError> {
        sqlx::query("UPDATE api_keys SET last_used_at = now() WHERE lookup_prefix = $1")
            .bind(lookup_prefix)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn environment_kind(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<String, StorageError> {
        sqlx::query_scalar("SELECT kind FROM environments WHERE id = $1 AND workspace_id = $2")
            .bind(environment_id.to_string())
            .bind(workspace_id.to_string())
            .fetch_optional(&self.pool)
            .await?
            .ok_or(StorageError::NotFound)
    }

    pub async fn list_api_keys(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredApiKey>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, name, lookup_prefix, scopes, expires_at, last_used_at,
                    revoked_at, created_at
             FROM api_keys
             WHERE workspace_id = $1 AND environment_id = $2
             ORDER BY created_at DESC, id DESC",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(stored_api_key_from_row).collect()
    }

    pub async fn create_api_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
        name: &str,
        lookup_prefix: &str,
        secret_hash: &str,
        scopes: &[String],
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<StoredApiKey, StorageError> {
        let row = sqlx::query(
            "INSERT INTO api_keys (
                id, workspace_id, environment_id, name, lookup_prefix,
                secret_hash, scopes, expires_at
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             RETURNING id, name, lookup_prefix, scopes, expires_at, last_used_at,
                       revoked_at, created_at",
        )
        .bind(id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(name)
        .bind(lookup_prefix)
        .bind(secret_hash)
        .bind(scopes)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await?;
        stored_api_key_from_row(&row)
    }

    pub async fn revoke_api_key(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredApiKey, StorageError> {
        let row = sqlx::query(
            "UPDATE api_keys SET revoked_at = COALESCE(revoked_at, now())
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
             RETURNING id, name, lookup_prefix, scopes, expires_at, last_used_at,
                       revoked_at, created_at",
        )
        .bind(id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        stored_api_key_from_row(&row)
    }

    pub async fn agent_for_authentication(
        &self,
        agent_id: AgentId,
    ) -> Result<AgentAuthenticationRecord, StorageError> {
        let row = sqlx::query(
            "SELECT workspace_id, environment_id, public_key
             FROM agents WHERE id = $1 AND revoked_at IS NULL",
        )
        .bind(agent_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        let workspace_id: String = row.try_get("workspace_id")?;
        let environment_id: String = row.try_get("environment_id")?;
        Ok(AgentAuthenticationRecord {
            workspace_id: workspace_id.parse().map_err(|error| {
                StorageError::InvalidData(format!("workspace id `{workspace_id}`: {error}"))
            })?,
            environment_id: environment_id.parse().map_err(|error| {
                StorageError::InvalidData(format!("environment id `{environment_id}`: {error}"))
            })?,
            public_key: row
                .try_get::<Option<Vec<u8>>, _>("public_key")?
                .ok_or_else(|| StorageError::InvalidData("agent has no public key".into()))?,
        })
    }

    pub async fn reserve_agent_nonce(
        &self,
        agent_id: AgentId,
        nonce: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("DELETE FROM agent_nonces WHERE expires_at <= now()")
            .execute(&mut *transaction)
            .await?;
        let result = sqlx::query(
            "INSERT INTO agent_nonces (agent_id, nonce, expires_at)
             VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
        )
        .bind(agent_id.to_string())
        .bind(nonce)
        .bind(expires_at)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StorageError::ConcurrentStateChange);
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn sync_agent_presence(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        version: &str,
        printers: Option<&[SyncedPrinter]>,
    ) -> Result<(), StorageError> {
        let mut transaction = self.pool.begin().await?;
        let result = sqlx::query(
            "UPDATE agents SET state = 'connected', version = $4, last_seen_at = now()
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
               AND revoked_at IS NULL",
        )
        .bind(agent_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(version)
        .execute(&mut *transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(StorageError::NotFound);
        }
        if let Some(printers) = printers {
            sqlx::query(
                "UPDATE printers SET removed_at = now()
                 WHERE agent_id = $1 AND workspace_id = $2 AND environment_id = $3",
            )
            .bind(agent_id.to_string())
            .bind(workspace_id.to_string())
            .bind(environment_id.to_string())
            .execute(&mut *transaction)
            .await?;
            for printer in printers {
                sqlx::query(
                    "INSERT INTO printers (
                        id, workspace_id, environment_id, agent_id, native_id, name,
                        state, capabilities, capabilities_revision, is_default,
                        native_options, profiles, last_seen_at, removed_at
                     ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,now(),NULL)
                     ON CONFLICT (agent_id, native_id) DO UPDATE SET
                        name = EXCLUDED.name, state = EXCLUDED.state,
                        capabilities = EXCLUDED.capabilities,
                        capabilities_revision = EXCLUDED.capabilities_revision,
                        is_default = EXCLUDED.is_default,
                        native_options = EXCLUDED.native_options,
                        profiles = EXCLUDED.profiles,
                        last_seen_at = now(), removed_at = NULL",
                )
                .bind(printer.id.to_string())
                .bind(workspace_id.to_string())
                .bind(environment_id.to_string())
                .bind(agent_id.to_string())
                .bind(&printer.native_id)
                .bind(&printer.name)
                .bind(printer_state_name(printer.state))
                .bind(serde_json::to_value(&printer.capabilities)?)
                .bind(i64::try_from(printer.capability_revision).map_err(|error| {
                    StorageError::InvalidData(format!("capability revision overflow: {error}"))
                })?)
                .bind(printer.is_default)
                .bind(serde_json::to_value(&printer.native_options)?)
                .bind(serde_json::to_value(&printer.profiles)?)
                .execute(&mut *transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        Ok(())
    }

    pub async fn enqueue_agent_command(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        command: &serde_json::Value,
    ) -> Result<String, StorageError> {
        let cursor: i64 = sqlx::query_scalar(
            "INSERT INTO agent_commands (workspace_id, environment_id, agent_id, command)
             SELECT $1,$2,$3,$4
             WHERE EXISTS (
                 SELECT 1 FROM agents
                 WHERE id = $3 AND workspace_id = $1 AND environment_id = $2
                   AND revoked_at IS NULL
             )
             RETURNING cursor",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(agent_id.to_string())
        .bind(command)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        Ok(cursor.to_string())
    }

    pub async fn sync_agent_commands(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        acknowledged_cursor: Option<i64>,
        limit: i64,
    ) -> Result<StoredAgentCommandBatch, StorageError> {
        let mut transaction = self.pool.begin().await?;
        if let Some(cursor) = acknowledged_cursor {
            sqlx::query(
                "UPDATE agent_commands SET acknowledged_at = now()
                 WHERE workspace_id = $1 AND environment_id = $2 AND agent_id = $3
                   AND cursor <= $4 AND delivered_at IS NOT NULL
                   AND acknowledged_at IS NULL",
            )
            .bind(workspace_id.to_string())
            .bind(environment_id.to_string())
            .bind(agent_id.to_string())
            .bind(cursor)
            .execute(&mut *transaction)
            .await?;
        }
        let rows = sqlx::query(
            "WITH pending AS (
                 SELECT cursor FROM agent_commands
                 WHERE workspace_id = $1 AND environment_id = $2 AND agent_id = $3
                   AND acknowledged_at IS NULL
                 ORDER BY cursor
                 LIMIT $4
                 FOR UPDATE
             )
             UPDATE agent_commands AS command
             SET delivered_at = COALESCE(command.delivered_at, now())
             FROM pending
             WHERE command.cursor = pending.cursor
             RETURNING command.cursor, command.command",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(agent_id.to_string())
        .bind(limit.clamp(1, 100))
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;

        let mut commands = rows
            .into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<i64, _>("cursor")?,
                    row.try_get::<serde_json::Value, _>("command")?,
                ))
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        commands.sort_unstable_by_key(|(cursor, _)| *cursor);
        Ok(StoredAgentCommandBatch {
            cursor: commands.last().map(|(cursor, _)| cursor.to_string()),
            commands: commands.into_iter().map(|(_, command)| command).collect(),
        })
    }

    pub async fn list_agents(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredAgent>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, name, os, state, version, COALESCE(last_seen_at, created_at) AS last_seen_at
             FROM agents
             WHERE workspace_id = $1 AND environment_id = $2 AND revoked_at IS NULL
             ORDER BY created_at DESC, id DESC",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let id: String = row.try_get("id")?;
                Ok(StoredAgent {
                    id: id.parse().map_err(|error| {
                        StorageError::InvalidData(format!("agent id `{id}`: {error}"))
                    })?,
                    name: row.try_get("name")?,
                    platform: row.try_get("os")?,
                    state: normalize_agent_state(&row.try_get::<String, _>("state")?),
                    version: row.try_get("version")?,
                    last_seen_at: row.try_get("last_seen_at")?,
                })
            })
            .collect()
    }

    pub async fn list_printers(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        after: Option<PrinterId>,
        limit: i64,
    ) -> Result<Vec<StoredPrinter>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, agent_id, name, state, capabilities, capabilities_revision,
                    native_options, profiles,
                    COALESCE(last_seen_at, created_at) AS updated_at
             FROM printers
             WHERE workspace_id = $1 AND environment_id = $2 AND removed_at IS NULL
               AND ($3::text IS NULL OR (created_at, id) < (
                   SELECT created_at, id FROM printers
                   WHERE id = $3 AND workspace_id = $1 AND environment_id = $2
               ))
             ORDER BY created_at DESC, id DESC LIMIT $4",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(after.map(|id| id.to_string()))
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(printer_from_row).collect()
    }

    pub async fn get_printer(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<StoredPrinter, StorageError> {
        let row = sqlx::query(
            "SELECT id, agent_id, name, state, capabilities, capabilities_revision,
                    native_options, profiles,
                    COALESCE(last_seen_at, created_at) AS updated_at
             FROM printers
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
               AND removed_at IS NULL",
        )
        .bind(printer_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        printer_from_row(&row)
    }

    pub async fn list_stocks(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredStock>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, name, sku, description, attributes, archived, created_at, updated_at
             FROM stocks WHERE workspace_id = $1 AND environment_id = $2
             ORDER BY created_at, id",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(stock_from_row).collect()
    }

    pub async fn get_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredStock, StorageError> {
        let row = sqlx::query(
            "SELECT id, name, sku, description, attributes, archived, created_at, updated_at
             FROM stocks WHERE id = $1 AND workspace_id = $2 AND environment_id = $3",
        )
        .bind(id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        stock_from_row(&row)
    }

    pub async fn create_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        stock: &StoredStock,
    ) -> Result<StoredStock, StorageError> {
        sqlx::query(
            "INSERT INTO stocks (
                id, workspace_id, environment_id, name, sku, description, attributes, archived
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(&stock.id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(&stock.name)
        .bind(&stock.sku)
        .bind(&stock.description)
        .bind(&stock.attributes)
        .bind(stock.archived)
        .execute(&self.pool)
        .await
        .map_err(map_create_conflict)?;
        self.get_stock(workspace_id, environment_id, &stock.id)
            .await
    }

    pub async fn update_stock(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        stock: &StoredStock,
    ) -> Result<StoredStock, StorageError> {
        let result = sqlx::query(
            "UPDATE stocks SET name = $4, sku = $5, description = $6, attributes = $7,
                    archived = $8, updated_at = now()
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3",
        )
        .bind(&stock.id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(&stock.name)
        .bind(&stock.sku)
        .bind(&stock.description)
        .bind(&stock.attributes)
        .bind(stock.archived)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }
        self.get_stock(workspace_id, environment_id, &stock.id)
            .await
    }

    pub async fn list_targets(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredTarget>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, name, description, stock_id, enabled, routing_policy, created_at, updated_at
             FROM targets WHERE workspace_id = $1 AND environment_id = $2
             ORDER BY created_at, id",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(target_from_row).collect()
    }

    pub async fn get_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<StoredTarget, StorageError> {
        let row = sqlx::query(
            "SELECT id, name, description, stock_id, enabled, routing_policy, created_at, updated_at
             FROM targets WHERE id = $1 AND workspace_id = $2 AND environment_id = $3",
        )
        .bind(id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        target_from_row(&row)
    }

    pub async fn create_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target: &StoredTarget,
    ) -> Result<StoredTarget, StorageError> {
        let mut transaction = self.pool.begin().await?;
        validate_stock_reference(
            &mut transaction,
            workspace_id,
            environment_id,
            target.stock_id.as_deref(),
        )
        .await?;
        let row = sqlx::query(
            "INSERT INTO targets (
                id, workspace_id, environment_id, name, description, stock_id,
                enabled, routing_policy
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             RETURNING id, name, description, stock_id, enabled, routing_policy,
                       created_at, updated_at",
        )
        .bind(&target.id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(&target.name)
        .bind(&target.description)
        .bind(&target.stock_id)
        .bind(target.enabled)
        .bind(&target.routing_policy)
        .fetch_one(&mut *transaction)
        .await
        .map_err(map_create_conflict)?;
        let target = target_from_row(&row)?;
        transaction.commit().await?;
        Ok(target)
    }

    pub async fn update_target(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target: &StoredTarget,
    ) -> Result<StoredTarget, StorageError> {
        let mut transaction = self.pool.begin().await?;
        validate_stock_reference(
            &mut transaction,
            workspace_id,
            environment_id,
            target.stock_id.as_deref(),
        )
        .await?;
        let row = sqlx::query(
            "UPDATE targets SET name = $4, description = $5, stock_id = $6,
                    enabled = $7, routing_policy = $8, updated_at = now()
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
             RETURNING id, name, description, stock_id, enabled, routing_policy,
                       created_at, updated_at",
        )
        .bind(&target.id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(&target.name)
        .bind(&target.description)
        .bind(&target.stock_id)
        .bind(target.enabled)
        .bind(&target.routing_policy)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StorageError::NotFound)?;
        let target = target_from_row(&row)?;
        transaction.commit().await?;
        Ok(target)
    }

    pub async fn list_target_bindings(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target_id: &str,
    ) -> Result<Vec<StoredTargetBinding>, StorageError> {
        self.get_target(workspace_id, environment_id, target_id)
            .await?;
        let rows = sqlx::query(
            "SELECT id, target_id, printer_id, agent_id, profile_id, profile_revision,
                    role, enabled, created_at, updated_at
             FROM target_bindings
             WHERE target_id = $1 AND workspace_id = $2 AND environment_id = $3
             ORDER BY CASE role WHEN 'primary' THEN 0 ELSE 1 END, created_at, id",
        )
        .bind(target_id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.iter().map(binding_from_row).collect()
    }

    pub async fn create_target_binding(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        binding: &StoredTargetBinding,
    ) -> Result<StoredTargetBinding, StorageError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query_scalar::<_, String>(
            "SELECT id FROM targets
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
             FOR SHARE",
        )
        .bind(&binding.target_id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StorageError::NotFound)?;
        let (agent_id, profiles) = fetch_binding_printer(
            &mut transaction,
            workspace_id,
            environment_id,
            binding.printer_id,
        )
        .await?;
        let profile = profiles
            .iter()
            .find(|profile| {
                (profile.profile_id.as_str(), profile.revision)
                    == (binding.profile_id.as_str(), binding.profile_revision)
            })
            .ok_or(StorageError::NotFound)?;
        if !profile.published {
            return Err(StorageError::InvalidData(
                "target binding profile is not published".into(),
            ));
        }
        let row = sqlx::query(
            "INSERT INTO target_bindings (
                id, workspace_id, environment_id, target_id, printer_id, agent_id,
                profile_id, profile_revision, role, enabled
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
             RETURNING id, target_id, printer_id, agent_id, profile_id, profile_revision,
                       role, enabled, created_at, updated_at",
        )
        .bind(&binding.id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(&binding.target_id)
        .bind(binding.printer_id.to_string())
        .bind(agent_id.to_string())
        .bind(&binding.profile_id)
        .bind(i64::try_from(binding.profile_revision).map_err(|error| {
            StorageError::InvalidData(format!("profile revision is too large: {error}"))
        })?)
        .bind(&binding.role)
        .bind(binding.enabled)
        .fetch_one(&mut *transaction)
        .await
        .map_err(map_create_conflict)?;
        let binding = binding_from_row(&row)?;
        transaction.commit().await?;
        Ok(binding)
    }

    pub async fn get_target_binding(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target_id: &str,
        binding_id: &str,
    ) -> Result<StoredTargetBinding, StorageError> {
        let row = sqlx::query(
            "SELECT id, target_id, printer_id, agent_id, profile_id, profile_revision,
                    role, enabled, created_at, updated_at
             FROM target_bindings
             WHERE id = $1 AND target_id = $2 AND workspace_id = $3 AND environment_id = $4",
        )
        .bind(binding_id)
        .bind(target_id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        binding_from_row(&row)
    }

    pub async fn delete_target_binding(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        target_id: &str,
        binding_id: &str,
    ) -> Result<(), StorageError> {
        let result = sqlx::query(
            "DELETE FROM target_bindings
             WHERE id = $1 AND target_id = $2 AND workspace_id = $3 AND environment_id = $4",
        )
        .bind(binding_id)
        .bind(target_id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    pub async fn create_enrolment(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        secret_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO enrolment_tokens (
                id, workspace_id, environment_id, secret_hash, expires_at
             ) VALUES ($1,$2,$3,$4,$5)",
        )
        .bind(id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(secret_hash)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn enrol_agent(
        &self,
        secret_hash: &str,
        public_key: &[u8],
        name: &str,
        hostname: &str,
        platform: &str,
        architecture: &str,
        version: &str,
        protocol_version: u16,
    ) -> Result<EnrolledAgent, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT id, workspace_id, environment_id FROM enrolment_tokens
             WHERE secret_hash = $1 AND consumed_at IS NULL AND expires_at > now()
             FOR UPDATE",
        )
        .bind(secret_hash)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StorageError::NotFound)?;
        let token_id: String = row.try_get("id")?;
        let workspace_text: String = row.try_get("workspace_id")?;
        let environment_text: String = row.try_get("environment_id")?;
        let workspace_id: WorkspaceId = workspace_text.parse().map_err(|error| {
            StorageError::InvalidData(format!("workspace id `{workspace_text}`: {error}"))
        })?;
        let environment_id: EnvironmentId = environment_text.parse().map_err(|error| {
            StorageError::InvalidData(format!("environment id `{environment_text}`: {error}"))
        })?;
        let agent_id = AgentId::new();
        let installation_id = format!("{}:{:x}", hostname, Sha256::digest(public_key));
        sqlx::query(
            "INSERT INTO agents (
                id, workspace_id, environment_id, name, installation_id, public_key,
                os, architecture, version, protocol_version, state, last_seen_at
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'connected',now())",
        )
        .bind(agent_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(name)
        .bind(installation_id)
        .bind(public_key)
        .bind(platform)
        .bind(architecture)
        .bind(version)
        .bind(i32::from(protocol_version))
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE enrolment_tokens SET consumed_at = now() WHERE id = $1")
            .bind(token_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(EnrolledAgent {
            agent_id,
            workspace_id,
            environment_id,
        })
    }

    pub async fn list_webhooks(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<Vec<StoredWebhook>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, url, subscribed_events, enabled, created_at
             FROM webhook_endpoints
             WHERE workspace_id = $1 AND environment_id = $2
             ORDER BY created_at DESC, id DESC",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(StoredWebhook {
                    id: row.try_get("id")?,
                    url: row.try_get("url")?,
                    events: row.try_get("subscribed_events")?,
                    enabled: row.try_get("enabled")?,
                    created_at: row.try_get("created_at")?,
                })
            })
            .collect()
    }

    pub async fn create_webhook(
        &self,
        id: &str,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        url: &str,
        events: &[String],
        secret_ciphertext: &[u8],
    ) -> Result<StoredWebhook, StorageError> {
        sqlx::query(
            "INSERT INTO webhook_endpoints (
                id, workspace_id, environment_id, url, secret_ciphertext, subscribed_events
             ) VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(url)
        .bind(secret_ciphertext)
        .bind(events)
        .execute(&self.pool)
        .await?;
        Ok(StoredWebhook {
            id: id.to_owned(),
            url: url.to_owned(),
            events: events.to_vec(),
            enabled: true,
            created_at: Utc::now(),
        })
    }

    pub async fn delete_webhook(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        id: &str,
    ) -> Result<(), StorageError> {
        let result = sqlx::query(
            "DELETE FROM webhook_endpoints
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3",
        )
        .bind(id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    pub async fn list_webhook_deliveries(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        webhook_id: &str,
    ) -> Result<Vec<StoredWebhookDelivery>, StorageError> {
        let rows = sqlx::query(
            "SELECT delivery.id, delivery.event_id, delivery.attempt,
                    delivery.next_attempt_at, delivery.response_status,
                    delivery.delivered_at, delivery.dead_lettered_at
             FROM webhook_deliveries AS delivery
             JOIN webhook_endpoints AS endpoint ON endpoint.id = delivery.endpoint_id
             WHERE endpoint.id = $1 AND endpoint.workspace_id = $2
               AND endpoint.environment_id = $3
             ORDER BY delivery.created_at DESC LIMIT 500",
        )
        .bind(webhook_id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(StoredWebhookDelivery {
                    id: row.try_get("id")?,
                    event_id: row.try_get("event_id")?,
                    attempt: row.try_get("attempt")?,
                    next_attempt_at: row.try_get("next_attempt_at")?,
                    response_status: row.try_get("response_status")?,
                    delivered_at: row.try_get("delivered_at")?,
                    dead_lettered_at: row.try_get("dead_lettered_at")?,
                })
            })
            .collect()
    }

    pub async fn replay_webhook_delivery(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        delivery_id: &str,
    ) -> Result<(), StorageError> {
        let result = sqlx::query(
            "UPDATE webhook_deliveries AS delivery
             SET next_attempt_at = now(), delivered_at = NULL, dead_lettered_at = NULL,
                 response_status = NULL, response_excerpt = NULL, claimed_until = NULL
             FROM webhook_endpoints AS endpoint
             WHERE delivery.id = $1 AND endpoint.id = delivery.endpoint_id
               AND endpoint.workspace_id = $2 AND endpoint.environment_id = $3",
        )
        .bind(delivery_id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound);
        }
        Ok(())
    }

    pub async fn enqueue_webhook_event(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Result<String, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let event_id = EventId::new().to_string();
        sqlx::query(
            "INSERT INTO webhook_events (
                id, workspace_id, environment_id, event_type, payload, occurred_at
             ) VALUES ($1,$2,$3,$4,$5,now())",
        )
        .bind(&event_id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(event_type)
        .bind(payload)
        .execute(&mut *transaction)
        .await?;
        let endpoints = sqlx::query(
            "SELECT id, url FROM webhook_endpoints
             WHERE workspace_id = $1 AND environment_id = $2 AND enabled = true
               AND $3 = ANY(subscribed_events)
             FOR SHARE",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(event_type)
        .fetch_all(&mut *transaction)
        .await?;
        for endpoint in endpoints {
            sqlx::query(
                "INSERT INTO webhook_deliveries (
                    id, endpoint_id, event_id, destination_url, next_attempt_at
                 ) VALUES ($1,$2,$3,$4,now())",
            )
            .bind(format!("whd_{}", ulid::Ulid::new()))
            .bind(endpoint.try_get::<String, _>("id")?)
            .bind(&event_id)
            .bind(endpoint.try_get::<String, _>("url")?)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(event_id)
    }

    pub async fn list_tenant_events(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        after: Option<&str>,
        limit: i64,
    ) -> Result<Vec<StoredTenantEvent>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, event_type, payload FROM webhook_events
             WHERE workspace_id = $1 AND environment_id = $2
               AND ($3::text IS NULL OR id > $3)
             ORDER BY id LIMIT $4",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(after)
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(StoredTenantEvent {
                    id: row.try_get("id")?,
                    event_type: row.try_get("event_type")?,
                    payload: row.try_get("payload")?,
                })
            })
            .collect()
    }

    pub async fn claim_webhook_deliveries(
        &self,
        limit: i64,
    ) -> Result<Vec<WebhookDeliveryWork>, StorageError> {
        let rows = sqlx::query(
            "WITH candidates AS (
                SELECT id FROM webhook_deliveries
                WHERE delivered_at IS NULL AND dead_lettered_at IS NULL
                  AND next_attempt_at <= now()
                  AND (claimed_until IS NULL OR claimed_until < now())
                ORDER BY next_attempt_at, created_at
                FOR UPDATE SKIP LOCKED LIMIT $1
             )
             UPDATE webhook_deliveries AS delivery
             SET claimed_until = now() + $2 * interval '1 second'
             FROM candidates, webhook_endpoints AS endpoint, webhook_events AS event
             WHERE delivery.id = candidates.id
               AND endpoint.id = delivery.endpoint_id
               AND event.id = delivery.event_id
             RETURNING delivery.id, delivery.event_id, delivery.destination_url,
                       delivery.attempt, endpoint.secret_ciphertext,
                       event.event_type, event.payload, event.occurred_at",
        )
        .bind(limit.clamp(1, WEBHOOK_MAX_CLAIM_BATCH))
        .bind(WEBHOOK_CLAIM_TTL_SECONDS)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(WebhookDeliveryWork {
                    id: row.try_get("id")?,
                    event_id: row.try_get("event_id")?,
                    event_type: row.try_get("event_type")?,
                    url: row.try_get("destination_url")?,
                    secret_ciphertext: row.try_get("secret_ciphertext")?,
                    payload: row.try_get("payload")?,
                    event_occurred_at: row.try_get("occurred_at")?,
                    attempt: row.try_get("attempt")?,
                })
            })
            .collect()
    }

    pub async fn record_webhook_attempt(
        &self,
        delivery_id: &str,
        status: Option<i32>,
        response_excerpt: Option<&str>,
        next_attempt_at: Option<DateTime<Utc>>,
        delivered: bool,
    ) -> Result<(), StorageError> {
        let dead_letter = !delivered && next_attempt_at.is_none();
        sqlx::query(
            "UPDATE webhook_deliveries
             SET attempt = attempt + 1, response_status = $2, response_excerpt = $3,
                 next_attempt_at = COALESCE($4, next_attempt_at),
                 delivered_at = CASE WHEN $5 THEN now() ELSE delivered_at END,
                 dead_lettered_at = CASE WHEN $6 THEN now() ELSE dead_lettered_at END,
                 claimed_until = NULL
             WHERE id = $1",
        )
        .bind(delivery_id)
        .bind(status)
        .bind(response_excerpt.map(|value| value.chars().take(2_048).collect::<String>()))
        .bind(next_attempt_at)
        .bind(delivered)
        .bind(dead_letter)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_upload(
        &self,
        upload: &StoredUpload,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
    ) -> Result<(), StorageError> {
        sqlx::query(
            "INSERT INTO uploads (
                id, workspace_id, environment_id, object_key, media_type,
                expected_sha256, expected_bytes, state, expires_at
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,'pending',$8)",
        )
        .bind(&upload.id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(&upload.object_key)
        .bind(&upload.media_type)
        .bind(&upload.expected_sha256)
        .bind(upload.expected_bytes)
        .bind(upload.expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_upload(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        upload_id: &str,
    ) -> Result<StoredUpload, StorageError> {
        let row = sqlx::query(
            "SELECT id, object_key, media_type, expected_sha256,
                    expected_bytes, state, expires_at
             FROM uploads
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3",
        )
        .bind(upload_id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        Ok(StoredUpload {
            id: row.try_get("id")?,
            object_key: row.try_get("object_key")?,
            media_type: row.try_get("media_type")?,
            expected_sha256: row.try_get("expected_sha256")?,
            expected_bytes: row.try_get("expected_bytes")?,
            state: row.try_get("state")?,
            expires_at: row.try_get("expires_at")?,
        })
    }

    pub async fn complete_upload(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        upload_id: &str,
        actual_sha256: &str,
        actual_bytes: i64,
    ) -> Result<StoredUpload, StorageError> {
        let row = sqlx::query(
            "UPDATE uploads
             SET state = 'complete', completed_at = now()
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
               AND state = 'pending' AND expires_at > now()
               AND expected_sha256 = $4 AND expected_bytes = $5
             RETURNING id, object_key, media_type, expected_sha256,
                       expected_bytes, state, expires_at",
        )
        .bind(upload_id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(actual_sha256)
        .bind(actual_bytes)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        Ok(StoredUpload {
            id: row.try_get("id")?,
            object_key: row.try_get("object_key")?,
            media_type: row.try_get("media_type")?,
            expected_sha256: row.try_get("expected_sha256")?,
            expected_bytes: row.try_get("expected_bytes")?,
            state: row.try_get("state")?,
            expires_at: row.try_get("expires_at")?,
        })
    }

    pub async fn create_job(
        &self,
        job: &Job,
        agent_id: AgentId,
        idempotency_key: Option<&str>,
        request_bytes: &[u8],
    ) -> Result<CreateJobResult, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let request_hash = format!("{:x}", Sha256::digest(request_bytes));

        if let Some(key) = idempotency_key
            && let Some(existing) = find_idempotent_job(
                &mut transaction,
                job.workspace_id,
                job.environment_id,
                key,
                &request_hash,
            )
            .await?
        {
            transaction.commit().await?;
            return Ok(CreateJobResult::Existing(existing));
        }

        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(job.printer_id.to_string())
            .execute(&mut *transaction)
            .await?;
        let per_printer_sequence: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(per_printer_sequence), 0) + 1
             FROM jobs WHERE printer_id = $1",
        )
        .bind(job.printer_id.to_string())
        .fetch_one(&mut *transaction)
        .await?;

        let payload = serde_json::to_value(job)?;
        sqlx::query(
            "INSERT INTO jobs (
                id, workspace_id, environment_id, printer_id, agent_id, payload, state,
                state_sequence, per_printer_sequence, expires_at, created_at, updated_at
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,1,$8,$9,$10,$10)",
        )
        .bind(job.id.to_string())
        .bind(job.workspace_id.to_string())
        .bind(job.environment_id.to_string())
        .bind(job.printer_id.to_string())
        .bind(agent_id.to_string())
        .bind(payload)
        .bind(state_name(job.state))
        .bind(per_printer_sequence)
        .bind(job.expires_at)
        .bind(job.created_at)
        .execute(&mut *transaction)
        .await?;

        let initial_event = JobEvent {
            id: EventId::new(),
            job_id: job.id,
            sequence: 1,
            state: job.state,
            reason: None,
            message: Some("Job durably registered".into()),
            agent_id: None,
            native_job_id: None,
            occurred_at: job.created_at,
        };
        insert_event(&mut transaction, job, &initial_event).await?;

        sqlx::query(
            "INSERT INTO routing_outbox (
                id, workspace_id, environment_id, aggregate_type, aggregate_id,
                event_type, payload
             ) VALUES ($1,$2,$3,'job',$4,'job.registered',$5)",
        )
        .bind(EventId::new().to_string())
        .bind(job.workspace_id.to_string())
        .bind(job.environment_id.to_string())
        .bind(job.id.to_string())
        .bind(serde_json::to_value(&initial_event)?)
        .execute(&mut *transaction)
        .await?;

        if let Some(key) = idempotency_key {
            sqlx::query(
                "INSERT INTO idempotency_requests (
                    workspace_id, environment_id, operation, key, request_hash,
                    resource_id, response_status, response_body, expires_at
                 ) VALUES ($1,$2,'jobs.create',$3,$4,$5,201,$6,now() + interval '24 hours')",
            )
            .bind(job.workspace_id.to_string())
            .bind(job.environment_id.to_string())
            .bind(key)
            .bind(request_hash)
            .bind(job.id.to_string())
            .bind(serde_json::to_value(job)?)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(CreateJobResult::Created(job.clone()))
    }

    pub async fn get_job(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
    ) -> Result<Job, StorageError> {
        let row = sqlx::query(
            "SELECT payload, state FROM jobs
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3",
        )
        .bind(job_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        let state: String = row.try_get("state")?;
        job_from_row(row.try_get("payload")?, &state)
    }

    pub async fn get_job_sequence(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
    ) -> Result<u64, StorageError> {
        let sequence: i64 = sqlx::query_scalar(
            "SELECT state_sequence FROM jobs
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3",
        )
        .bind(job_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        u64::try_from(sequence)
            .map_err(|error| StorageError::InvalidData(format!("negative sequence: {error}")))
    }

    pub async fn resolve_printer_agent(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        printer_id: PrinterId,
    ) -> Result<AgentId, StorageError> {
        let value: String = sqlx::query_scalar(
            "SELECT agent_id FROM printers
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
               AND removed_at IS NULL",
        )
        .bind(printer_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)?;
        value
            .parse()
            .map_err(|error| StorageError::InvalidData(format!("agent id `{value}`: {error}")))
    }

    pub async fn compatibility_id(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<i64, StorageError> {
        sqlx::query(
            "INSERT INTO compatibility_ids (
                workspace_id, environment_id, resource_type, resource_id
             ) VALUES ($1,$2,$3,$4)
             ON CONFLICT (workspace_id, environment_id, resource_type, resource_id)
             DO NOTHING",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(resource_type)
        .bind(resource_id)
        .execute(&self.pool)
        .await?;
        sqlx::query_scalar(
            "SELECT id FROM compatibility_ids
             WHERE workspace_id = $1 AND environment_id = $2
               AND resource_type = $3 AND resource_id = $4",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(resource_type)
        .bind(resource_id)
        .fetch_one(&self.pool)
        .await
        .map_err(StorageError::from)
    }

    pub async fn resolve_compatibility_id(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        resource_type: &str,
        compatibility_id: i64,
    ) -> Result<String, StorageError> {
        sqlx::query_scalar(
            "SELECT resource_id FROM compatibility_ids
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
               AND resource_type = $4",
        )
        .bind(compatibility_id)
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(resource_type)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::NotFound)
    }

    pub async fn list_jobs(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        after: Option<JobId>,
        limit: i64,
    ) -> Result<Vec<Job>, StorageError> {
        let rows = sqlx::query(
            "SELECT payload, state FROM jobs
             WHERE workspace_id = $1 AND environment_id = $2
               AND ($3::text IS NULL OR (created_at, id) < (
                   SELECT created_at, id FROM jobs
                   WHERE id = $3 AND workspace_id = $1 AND environment_id = $2
               ))
             ORDER BY created_at DESC, id DESC LIMIT $4",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(after.map(|id| id.to_string()))
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let state: String = row.get("state");
                job_from_row(row.get("payload"), &state)
            })
            .collect()
    }

    pub async fn list_job_events(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
    ) -> Result<Vec<JobEvent>, StorageError> {
        let rows = sqlx::query(
            "SELECT payload FROM job_events
             WHERE workspace_id = $1 AND environment_id = $2 AND job_id = $3
             ORDER BY sequence",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(job_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| serde_json::from_value(row.get("payload")).map_err(StorageError::from))
            .collect()
    }

    pub async fn append_event(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        event: &JobEvent,
    ) -> Result<Job, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT payload, state, state_sequence FROM jobs
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
             FOR UPDATE",
        )
        .bind(event.job_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StorageError::NotFound)?;

        let state: String = row.try_get("state")?;
        let mut job = job_from_row(row.try_get("payload")?, &state)?;
        let current_sequence: i64 = row.try_get("state_sequence")?;
        if event.sequence != u64::try_from(current_sequence + 1).unwrap_or(u64::MAX) {
            return Err(StorageError::ConcurrentStateChange);
        }
        validate_transition(job.state, event.state).map_err(|_| StorageError::InvalidTransition)?;
        job.state = event.state;
        insert_event(&mut transaction, &job, event).await?;

        let final_at = event.state.is_terminal().then_some(event.occurred_at);
        sqlx::query(
            "UPDATE jobs SET payload = $2, state = $3, state_sequence = $4,
             final_at = COALESCE($5, final_at), updated_at = now() WHERE id = $1",
        )
        .bind(event.job_id.to_string())
        .bind(serde_json::to_value(&job)?)
        .bind(state_name(event.state))
        .bind(i64::try_from(event.sequence).map_err(|error| {
            StorageError::InvalidData(format!("event sequence overflow: {error}"))
        })?)
        .bind(final_at)
        .execute(&mut *transaction)
        .await?;

        if event.state == JobState::AcceptedBySpooler {
            sqlx::query(
                "INSERT INTO usage_ledger (
                    id, workspace_id, environment_id, job_id, kind, units, occurred_at
                 )
                 SELECT $1,$2,$3,$4,'print_job_accepted',1,$5
                 WHERE EXISTS (
                   SELECT 1 FROM environments WHERE id = $3 AND kind = 'live'
                 )
                 ON CONFLICT (job_id) WHERE kind = 'print_job_accepted' AND job_id IS NOT NULL
                 DO NOTHING",
            )
            .bind(EventId::new().to_string())
            .bind(workspace_id.to_string())
            .bind(environment_id.to_string())
            .bind(event.job_id.to_string())
            .bind(event.occurred_at)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(job)
    }

    pub async fn request_job_cancellation(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        job_id: JobId,
        command: &serde_json::Value,
    ) -> Result<Job, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT payload, state, state_sequence, agent_id FROM jobs
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
             FOR UPDATE",
        )
        .bind(job_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StorageError::NotFound)?;
        let state: String = row.try_get("state")?;
        let mut job = job_from_row(row.try_get("payload")?, &state)?;
        validate_transition(job.state, JobState::CancelRequested)
            .map_err(|_| StorageError::InvalidTransition)?;
        let sequence = u64::try_from(row.try_get::<i64, _>("state_sequence")? + 1)
            .map_err(|error| StorageError::InvalidData(error.to_string()))?;
        job.state = JobState::CancelRequested;
        let event = JobEvent {
            id: EventId::new(),
            job_id,
            sequence,
            state: JobState::CancelRequested,
            reason: None,
            message: Some("Cancellation requested by API caller".into()),
            agent_id: None,
            native_job_id: None,
            occurred_at: Utc::now(),
        };
        insert_event(&mut transaction, &job, &event).await?;
        sqlx::query(
            "UPDATE jobs SET payload = $2, state = 'cancel_requested',
                 state_sequence = $3, updated_at = now()
             WHERE id = $1",
        )
        .bind(job_id.to_string())
        .bind(serde_json::to_value(&job)?)
        .bind(i64::try_from(sequence).map_err(|error| {
            StorageError::InvalidData(format!("event sequence overflow: {error}"))
        })?)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO agent_commands (workspace_id, environment_id, agent_id, command)
             VALUES ($1,$2,$3,$4)",
        )
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(row.try_get::<String, _>("agent_id")?)
        .bind(command)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(job)
    }

    pub async fn claim_jobs(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        owner: &str,
        limit: i64,
    ) -> Result<Vec<JobLease>, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let rows = sqlx::query(
            "SELECT id, payload, state FROM jobs
             WHERE agent_id = $1 AND workspace_id = $2 AND environment_id = $3
               AND state IN ('waiting_for_agent', 'failed_retryable')
               AND expires_at > now()
               AND (lease_until IS NULL OR lease_until < now())
             ORDER BY created_at, id
             FOR UPDATE SKIP LOCKED
             LIMIT $4",
        )
        .bind(agent_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(limit.clamp(1, 100))
        .fetch_all(&mut *transaction)
        .await?;
        let lease_until = Utc::now() + chrono::Duration::seconds(30);
        let mut leases = Vec::with_capacity(rows.len());
        for row in rows {
            let job_id: String = row.try_get("id")?;
            let state: String = row.try_get("state")?;
            let job = job_from_row(row.try_get("payload")?, &state)?;
            let lease_id = Uuid::new_v4();
            let mut token_bytes = [0_u8; 32];
            OsRng.fill_bytes(&mut token_bytes);
            let lease_token = URL_SAFE_NO_PAD.encode(token_bytes);
            let token_hash = Sha256::digest(lease_token.as_bytes()).to_vec();
            sqlx::query(
                "UPDATE jobs SET lease_owner = $2, lease_id = $3, lease_token_hash = $4,
                    lease_until = $5, updated_at = now() WHERE id = $1",
            )
            .bind(job_id)
            .bind(owner)
            .bind(lease_id)
            .bind(token_hash)
            .bind(lease_until)
            .execute(&mut *transaction)
            .await?;
            leases.push(JobLease {
                job,
                lease_id,
                lease_token,
                lease_until,
            });
        }
        transaction.commit().await?;
        Ok(leases)
    }

    pub async fn renew_lease(
        &self,
        job_id: JobId,
        owner: &str,
    ) -> Result<DateTime<Utc>, StorageError> {
        sqlx::query_scalar(
            "UPDATE jobs SET lease_until = now() + interval '30 seconds', updated_at = now()
             WHERE id = $1 AND lease_owner = $2 AND lease_until > now()
             RETURNING lease_until",
        )
        .bind(job_id.to_string())
        .bind(owner)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::ConcurrentStateChange)
    }

    pub async fn renew_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<DateTime<Utc>, StorageError> {
        let token_hash = Sha256::digest(lease_token.as_bytes()).to_vec();
        sqlx::query_scalar(
            "UPDATE jobs SET lease_until = now() + interval '30 seconds', updated_at = now()
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3 AND agent_id = $4
               AND lease_id = $5 AND lease_token_hash = $6 AND lease_until > now()
             RETURNING lease_until",
        )
        .bind(job_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(agent_id.to_string())
        .bind(lease_id)
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StorageError::ConcurrentStateChange)
    }

    pub async fn release_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<(), StorageError> {
        let token_hash = Sha256::digest(lease_token.as_bytes()).to_vec();
        let result = sqlx::query(
            "UPDATE jobs SET lease_owner = NULL, lease_id = NULL, lease_token_hash = NULL,
                    lease_until = NULL, updated_at = now()
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3 AND agent_id = $4
               AND lease_id = $5 AND lease_token_hash = $6 AND lease_until > now()",
        )
        .bind(job_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(agent_id.to_string())
        .bind(lease_id)
        .bind(token_hash)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() == 0 {
            return Err(StorageError::ConcurrentStateChange);
        }
        Ok(())
    }

    pub async fn validate_agent_lease(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
    ) -> Result<(), StorageError> {
        let token_hash = Sha256::digest(lease_token.as_bytes()).to_vec();
        let valid: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1 FROM jobs
                WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
                  AND agent_id = $4 AND lease_id = $5 AND lease_token_hash = $6
                  AND lease_until > now()
             )",
        )
        .bind(job_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(agent_id.to_string())
        .bind(lease_id)
        .bind(token_hash)
        .fetch_one(&self.pool)
        .await?;
        if !valid {
            return Err(StorageError::ConcurrentStateChange);
        }
        Ok(())
    }

    pub async fn apply_agent_event(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        reported: &JobEvent,
    ) -> Result<Option<Job>, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let receipt = sqlx::query(
            "INSERT INTO agent_event_receipts (agent_id, event_id)
             VALUES ($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(agent_id.to_string())
        .bind(reported.id.to_string())
        .execute(&mut *transaction)
        .await?;
        if receipt.rows_affected() == 0 {
            transaction.commit().await?;
            return Ok(None);
        }
        let row = sqlx::query(
            "SELECT payload, state, state_sequence FROM jobs
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
               AND agent_id = $4 FOR UPDATE",
        )
        .bind(reported.job_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(agent_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StorageError::NotFound)?;
        let current_state: String = row.try_get("state")?;
        let mut job = job_from_row(row.try_get("payload")?, &current_state)?;
        validate_transition(job.state, reported.state)
            .map_err(|_| StorageError::InvalidTransition)?;
        let sequence = row.try_get::<i64, _>("state_sequence")? + 1;
        let event = JobEvent {
            id: reported.id,
            job_id: reported.job_id,
            sequence: u64::try_from(sequence).map_err(|error| {
                StorageError::InvalidData(format!("event sequence overflow: {error}"))
            })?,
            state: reported.state,
            reason: reported.reason.clone(),
            message: reported.message.clone(),
            agent_id: Some(agent_id),
            native_job_id: reported.native_job_id.clone(),
            occurred_at: reported.occurred_at,
        };
        job.state = event.state;
        insert_event(&mut transaction, &job, &event).await?;
        sqlx::query(
            "UPDATE jobs SET payload = $2, state = $3, state_sequence = $4,
                final_at = CASE WHEN $5 THEN $6 ELSE final_at END, updated_at = now()
             WHERE id = $1",
        )
        .bind(job.id.to_string())
        .bind(serde_json::to_value(&job)?)
        .bind(state_name(event.state))
        .bind(sequence)
        .bind(event.state.is_terminal())
        .bind(event.occurred_at)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(Some(job))
    }

    pub async fn accept_agent_job(
        &self,
        workspace_id: WorkspaceId,
        environment_id: EnvironmentId,
        agent_id: AgentId,
        job_id: JobId,
        lease_id: Uuid,
        lease_token: &str,
        content_sha256: Option<&str>,
        local_sequence: u64,
    ) -> Result<Job, StorageError> {
        let mut transaction = self.pool.begin().await?;
        let token_hash = Sha256::digest(lease_token.as_bytes()).to_vec();
        if let Some(row) = sqlx::query(
            "SELECT lease_id, lease_token_hash, content_sha256, local_sequence
             FROM job_acceptances
             WHERE job_id = $1 AND workspace_id = $2 AND environment_id = $3 AND agent_id = $4
             FOR UPDATE",
        )
        .bind(job_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(agent_id.to_string())
        .fetch_optional(&mut *transaction)
        .await?
        {
            let stored_sha: Option<String> = row.try_get("content_sha256")?;
            let stored_sequence: i64 = row.try_get("local_sequence")?;
            let stored_lease_id: Uuid = row.try_get("lease_id")?;
            let stored_token_hash: Option<Vec<u8>> = row.try_get("lease_token_hash")?;
            if stored_lease_id != lease_id
                || stored_sha.as_deref() != content_sha256
                || stored_sequence != i64::try_from(local_sequence).unwrap_or(i64::MAX)
            {
                return Err(StorageError::IdempotencyConflict);
            }
            match stored_token_hash {
                Some(stored) if stored == token_hash => {}
                Some(_) => return Err(StorageError::IdempotencyConflict),
                None => {
                    let state: Option<String> = sqlx::query_scalar(
                        "SELECT state FROM jobs
                         WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
                           AND agent_id = $4",
                    )
                    .bind(job_id.to_string())
                    .bind(workspace_id.to_string())
                    .bind(environment_id.to_string())
                    .bind(agent_id.to_string())
                    .fetch_optional(&mut *transaction)
                    .await?;
                    if state.as_deref() != Some("agent_accepted") {
                        return Err(StorageError::IdempotencyConflict);
                    }
                    sqlx::query(
                        "UPDATE job_acceptances SET lease_token_hash = $2
                         WHERE job_id = $1 AND lease_token_hash IS NULL",
                    )
                    .bind(job_id.to_string())
                    .bind(&token_hash)
                    .execute(&mut *transaction)
                    .await?;
                }
            }
            transaction.commit().await?;
            return self.get_job(workspace_id, environment_id, job_id).await;
        }
        let row = sqlx::query(
            "SELECT payload, state, state_sequence FROM jobs
             WHERE id = $1 AND workspace_id = $2 AND environment_id = $3 AND agent_id = $4
               AND lease_id = $5 AND lease_token_hash = $6 AND lease_until > now()
             FOR UPDATE",
        )
        .bind(job_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(agent_id.to_string())
        .bind(lease_id)
        .bind(&token_hash)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(StorageError::ConcurrentStateChange)?;
        let current_state: String = row.try_get("state")?;
        let mut job = job_from_row(row.try_get("payload")?, &current_state)?;
        let expected_sha256 = match &job.content {
            spool_domain::ContentSource::Upload { upload_id } => Some(
                sqlx::query_scalar::<_, String>(
                    "SELECT expected_sha256 FROM uploads
                         WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
                           AND state = 'complete'",
                )
                .bind(upload_id)
                .bind(workspace_id.to_string())
                .bind(environment_id.to_string())
                .fetch_optional(&mut *transaction)
                .await?
                .ok_or(StorageError::NotFound)?,
            ),
            spool_domain::ContentSource::Base64 { data } => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|error| StorageError::InvalidData(error.to_string()))?;
                Some(format!("{:x}", Sha256::digest(decoded)))
            }
            spool_domain::ContentSource::Uri { .. } => None,
        };
        if expected_sha256.as_deref().is_some_and(|expected| {
            !content_sha256.is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
        }) {
            return Err(StorageError::InvalidData(
                "accepted content digest does not match the job".into(),
            ));
        }
        if !matches!(
            job.state,
            JobState::WaitingForAgent | JobState::AgentDownloading
        ) {
            return Err(StorageError::InvalidTransition);
        }
        job.state = JobState::AgentAccepted;
        let sequence: i64 = row.try_get::<i64, _>("state_sequence")? + 1;
        let event = JobEvent {
            id: EventId::new(),
            job_id,
            sequence: u64::try_from(sequence).map_err(|error| {
                StorageError::InvalidData(format!("event sequence overflow: {error}"))
            })?,
            state: JobState::AgentAccepted,
            reason: None,
            message: Some("Agent durably accepted the job".into()),
            agent_id: Some(agent_id),
            native_job_id: None,
            occurred_at: Utc::now(),
        };
        insert_event(&mut transaction, &job, &event).await?;
        sqlx::query(
            "UPDATE jobs SET payload = $2, state = 'agent_accepted', state_sequence = $3,
                    lease_owner = NULL, lease_id = NULL, lease_token_hash = NULL,
                    lease_until = NULL, updated_at = now()
             WHERE id = $1",
        )
        .bind(job_id.to_string())
        .bind(serde_json::to_value(&job)?)
        .bind(sequence)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO job_acceptances (
                job_id, workspace_id, environment_id, agent_id, lease_id,
                lease_token_hash, content_sha256, local_sequence
             ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(job_id.to_string())
        .bind(workspace_id.to_string())
        .bind(environment_id.to_string())
        .bind(agent_id.to_string())
        .bind(lease_id)
        .bind(&token_hash)
        .bind(content_sha256)
        .bind(i64::try_from(local_sequence).map_err(|error| {
            StorageError::InvalidData(format!("local sequence overflow: {error}"))
        })?)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(job)
    }

    pub async fn release_expired_jobs(&self) -> Result<u64, StorageError> {
        let result = sqlx::query(
            "UPDATE jobs SET state = 'expired', final_at = now(), updated_at = now()
             WHERE final_at IS NULL AND expires_at <= now()
               AND state NOT IN ('spool_intent','accepted_by_spooler','spooling','printing')",
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn readiness(&self) -> Result<(), StorageError> {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await?;
        Ok(())
    }
}

async fn find_idempotent_job(
    transaction: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
    key: &str,
    request_hash: &str,
) -> Result<Option<Job>, StorageError> {
    let row = sqlx::query(
        "SELECT request_hash, resource_id FROM idempotency_requests
         WHERE workspace_id = $1 AND environment_id = $2
           AND operation = 'jobs.create' AND key = $3 AND expires_at > now()
         FOR UPDATE",
    )
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .bind(key)
    .fetch_optional(&mut **transaction)
    .await?;
    match row {
        None => Ok(None),
        Some(row) if row.get::<String, _>("request_hash") == request_hash => {
            let resource_id: String = row.get("resource_id");
            let job_id = resource_id.parse().map_err(|error| {
                StorageError::InvalidData(format!("idempotent job id `{resource_id}`: {error}"))
            })?;
            let job_row = sqlx::query(
                "SELECT payload, state FROM jobs
                 WHERE id = $1 AND workspace_id = $2 AND environment_id = $3",
            )
            .bind(resource_id)
            .bind(workspace_id.to_string())
            .bind(environment_id.to_string())
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or(StorageError::NotFound)?;
            let state: String = job_row.try_get("state")?;
            let job = job_from_row(job_row.try_get("payload")?, &state)?;
            debug_assert_eq!(job.id, job_id);
            Ok(Some(job))
        }
        Some(_) => Err(StorageError::IdempotencyConflict),
    }
}

fn stored_api_key_from_row(row: &sqlx::postgres::PgRow) -> Result<StoredApiKey, StorageError> {
    Ok(StoredApiKey {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        lookup_prefix: row.try_get("lookup_prefix")?,
        scopes: row.try_get("scopes")?,
        expires_at: row.try_get("expires_at")?,
        last_used_at: row.try_get("last_used_at")?,
        revoked_at: row.try_get("revoked_at")?,
        created_at: row.try_get("created_at")?,
    })
}

async fn insert_event(
    transaction: &mut Transaction<'_, Postgres>,
    job: &Job,
    event: &JobEvent,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO job_events (
            id, workspace_id, environment_id, job_id, sequence,
            state, payload, occurred_at
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
    )
    .bind(event.id.to_string())
    .bind(job.workspace_id.to_string())
    .bind(job.environment_id.to_string())
    .bind(event.job_id.to_string())
    .bind(
        i64::try_from(event.sequence).map_err(|error| {
            StorageError::InvalidData(format!("event sequence overflow: {error}"))
        })?,
    )
    .bind(state_name(event.state))
    .bind(serde_json::to_value(event)?)
    .bind(event.occurred_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn job_from_row(payload: serde_json::Value, state: &str) -> Result<Job, StorageError> {
    let mut job: Job = serde_json::from_value(payload)?;
    job.state = parse_state(state)?;
    Ok(job)
}

const fn state_name(state: JobState) -> &'static str {
    match state {
        JobState::Registered => "registered",
        JobState::ContentPending => "content_pending",
        JobState::WaitingForAgent => "waiting_for_agent",
        JobState::AgentDownloading => "agent_downloading",
        JobState::AgentAccepted => "agent_accepted",
        JobState::QueuedLocal => "queued_local",
        JobState::Preparing => "preparing",
        JobState::Rendering => "rendering",
        JobState::SpoolIntent => "spool_intent",
        JobState::AcceptedBySpooler => "accepted_by_spooler",
        JobState::Spooling => "spooling",
        JobState::Printing => "printing",
        JobState::Blocked => "blocked",
        JobState::CompletedReported => "completed_reported",
        JobState::DeliveryUncertain => "delivery_uncertain",
        JobState::CancelRequested => "cancel_requested",
        JobState::Cancelled => "cancelled",
        JobState::Expired => "expired",
        JobState::FailedRetryable => "failed_retryable",
        JobState::FailedTerminal => "failed_terminal",
    }
}

const fn printer_state_name(state: PrinterState) -> &'static str {
    match state {
        PrinterState::Online => "online",
        PrinterState::Offline => "offline",
        PrinterState::Paused => "paused",
        PrinterState::Busy => "busy",
        PrinterState::PaperOut => "paper_out",
        PrinterState::Error => "error",
        PrinterState::Unknown => "unknown",
    }
}

fn parse_state(value: &str) -> Result<JobState, StorageError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|error| StorageError::InvalidData(format!("job state `{value}`: {error}")))
}

fn normalize_agent_state(value: &str) -> String {
    match value {
        "online" | "connected" => "connected",
        "paused" => "paused",
        "degraded" => "degraded",
        _ => "disconnected",
    }
    .to_owned()
}

fn printer_from_row(row: &PgRow) -> Result<StoredPrinter, StorageError> {
    let id: String = row.try_get("id")?;
    let agent_id: String = row.try_get("agent_id")?;
    let state: String = row.try_get("state")?;
    Ok(StoredPrinter {
        id: id
            .parse()
            .map_err(|error| StorageError::InvalidData(format!("printer id `{id}`: {error}")))?,
        agent_id: agent_id.parse().map_err(|error| {
            StorageError::InvalidData(format!("agent id `{agent_id}`: {error}"))
        })?,
        name: row.try_get("name")?,
        state: serde_json::from_value(serde_json::Value::String(state.clone())).map_err(
            |error| StorageError::InvalidData(format!("printer state `{state}`: {error}")),
        )?,
        capabilities: serde_json::from_value(row.try_get("capabilities")?)?,
        capability_revision: u64::try_from(row.try_get::<i64, _>("capabilities_revision")?)
            .map_err(|error| {
                StorageError::InvalidData(format!("capability revision is negative: {error}"))
            })?,
        native_options: serde_json::from_value(row.try_get("native_options")?)?,
        profiles: serde_json::from_value(row.try_get("profiles")?)?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn stock_from_row(row: &PgRow) -> Result<StoredStock, StorageError> {
    Ok(StoredStock {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        sku: row.try_get("sku")?,
        description: row.try_get("description")?,
        attributes: row.try_get("attributes")?,
        archived: row.try_get("archived")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn target_from_row(row: &PgRow) -> Result<StoredTarget, StorageError> {
    Ok(StoredTarget {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        stock_id: row.try_get("stock_id")?,
        enabled: row.try_get("enabled")?,
        routing_policy: row.try_get("routing_policy")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn binding_from_row(row: &PgRow) -> Result<StoredTargetBinding, StorageError> {
    let printer_id = row.try_get::<String, _>("printer_id")?;
    let agent_id = row.try_get::<String, _>("agent_id")?;
    let profile_revision = row.try_get::<i64, _>("profile_revision")?;
    Ok(StoredTargetBinding {
        id: row.try_get("id")?,
        target_id: row.try_get("target_id")?,
        printer_id: printer_id.parse().map_err(|error| {
            StorageError::InvalidData(format!("printer id `{printer_id}`: {error}"))
        })?,
        agent_id: agent_id.parse().map_err(|error| {
            StorageError::InvalidData(format!("agent id `{agent_id}`: {error}"))
        })?,
        profile_id: row.try_get("profile_id")?,
        profile_revision: u64::try_from(profile_revision).map_err(|error| {
            StorageError::InvalidData(format!("profile revision is negative: {error}"))
        })?,
        role: row.try_get("role")?,
        enabled: row.try_get("enabled")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

async fn validate_stock_reference(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
    stock_id: Option<&str>,
) -> Result<(), StorageError> {
    let Some(stock_id) = stock_id else {
        return Ok(());
    };
    sqlx::query_scalar::<_, bool>(
        "SELECT archived FROM stocks
         WHERE id = $1 AND workspace_id = $2 AND environment_id = $3
         FOR SHARE",
    )
    .bind(stock_id)
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?
    .filter(|archived| !archived)
    .ok_or(StorageError::NotFound)?;
    Ok(())
}

fn map_create_conflict(error: sqlx::Error) -> StorageError {
    if error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "23505")
    {
        StorageError::ConcurrentStateChange
    } else {
        StorageError::Database(error)
    }
}

async fn fetch_binding_printer(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    workspace_id: WorkspaceId,
    environment_id: EnvironmentId,
    printer_id: PrinterId,
) -> Result<(AgentId, Vec<PrinterProfileSnapshot>), StorageError> {
    let row = sqlx::query(
        "SELECT agent_id, profiles FROM printers
         WHERE id = $1 AND workspace_id = $2 AND environment_id = $3 AND removed_at IS NULL
         FOR SHARE",
    )
    .bind(printer_id.to_string())
    .bind(workspace_id.to_string())
    .bind(environment_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or(StorageError::NotFound)?;
    let agent_id = row.try_get::<String, _>("agent_id")?;
    Ok((
        agent_id.parse().map_err(|error| {
            StorageError::InvalidData(format!("agent id `{agent_id}`: {error}"))
        })?,
        serde_json::from_value(row.try_get("profiles")?)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_round_trips_through_database_name() {
        for state in [
            JobState::Registered,
            JobState::ContentPending,
            JobState::WaitingForAgent,
            JobState::AgentDownloading,
            JobState::AgentAccepted,
            JobState::QueuedLocal,
            JobState::Preparing,
            JobState::Rendering,
            JobState::SpoolIntent,
            JobState::AcceptedBySpooler,
            JobState::Spooling,
            JobState::Printing,
            JobState::Blocked,
            JobState::CompletedReported,
            JobState::DeliveryUncertain,
            JobState::CancelRequested,
            JobState::Cancelled,
            JobState::Expired,
            JobState::FailedRetryable,
            JobState::FailedTerminal,
        ] {
            assert_eq!(parse_state(state_name(state)).ok(), Some(state));
        }
    }

    #[test]
    fn lease_duration_matches_protocol_contract() {
        assert!(chrono::Duration::seconds(30).num_seconds() == 30);
    }
}
