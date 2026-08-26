//! Revocable application-scoped authorization for the local node broker.

use anyhow::{Context as _, Result, bail};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::command::{CommandFailure, RuntimeCommand};
use piqae_local_ipc::{
    BROKER_PROOF_MAX_SKEW_MS, BROKER_PROTOCOL_MIN_VERSION, BROKER_PROTOCOL_VERSION,
    BrokerApplicationIdentity, BrokerAuthorizationDecision, BrokerAuthorizationHandle,
    BrokerAuthorizationState, BrokerCapability, BrokerCredential, BrokerOperation, BrokerPresence,
    BrokerRequest, BrokerResponse, BrokerResult, LocalFailure, LocalOperation, LocalResult,
    PendingBrokerAuthorization, SdkBrokerOperation, broker_request_proof, broker_response_proof,
    constant_time_proof_eq, read_message, write_message,
};

const DOCUMENT_VERSION: u16 = 2;
const MAX_APPLICATIONS: usize = 128;
const MAX_PENDING_AUTHORIZATIONS: usize = 64;
const AUTHORIZATION_LIFETIME_MS: i64 = 5 * 60 * 1_000;
const MAX_BROKER_CONNECTIONS: usize = 32;
const BROKER_IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const MAX_REPLAY_PROOFS: usize = 4_096;

pub type ApplicationIdentity = BrokerApplicationIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "serialized least-privilege capabilities are independent grants"
)]
pub struct ApplicationCapabilities {
    pub observe_status: bool,
    pub observe_printers: bool,
    pub manage_profiles: bool,
    pub submit_local_jobs: bool,
    pub manage_connectors: bool,
}

impl ApplicationCapabilities {
    pub const OBSERVE_ONLY: Self = Self {
        observe_status: true,
        observe_printers: true,
        manage_profiles: false,
        submit_local_jobs: false,
        manage_connectors: false,
    };

    #[must_use]
    pub const fn allows(&self, requested: Self) -> bool {
        (!requested.observe_status || self.observe_status)
            && (!requested.observe_printers || self.observe_printers)
            && (!requested.manage_profiles || self.manage_profiles)
            && (!requested.submit_local_jobs || self.submit_local_jobs)
            && (!requested.manage_connectors || self.manage_connectors)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct BrokerToken(String);

impl BrokerToken {
    #[must_use]
    pub fn expose_for_client(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for BrokerToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BrokerToken([REDACTED])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationAuthorization {
    pub identity: ApplicationIdentity,
    pub capabilities: ApplicationCapabilities,
    pub token: BrokerToken,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableApplicationAuthorization {
    identity: ApplicationIdentity,
    capabilities: ApplicationCapabilities,
    token_sha256: String,
    revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableReplayProof {
    application_id: String,
    nonce_sha256: String,
    expires_unix_ms: i64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrokerDocument {
    version: u16,
    applications: Vec<DurableApplicationAuthorization>,
    #[serde(default)]
    replay_proofs: Vec<DurableReplayProof>,
}

#[derive(Debug)]
pub struct BrokerRegistry {
    root: PathBuf,
    applications: BTreeMap<String, DurableApplicationAuthorization>,
    replay_proofs: BTreeMap<(String, String), i64>,
}

impl BrokerRegistry {
    /// Loads the bounded application authorization registry.
    ///
    /// # Errors
    ///
    /// Fails closed for malformed, unsupported or unbounded state.
    pub fn load(root: impl AsRef<std::path::Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let path = root.join("broker-applications.json");
        let document = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<BrokerDocument>(&bytes)
                .with_context(|| format!("parse {}", path.display()))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BrokerDocument {
                version: DOCUMENT_VERSION,
                applications: Vec::new(),
                replay_proofs: Vec::new(),
            },
            Err(error) => return Err(error).with_context(|| format!("read {}", path.display())),
        };
        if !matches!(document.version, 1 | DOCUMENT_VERSION) {
            bail!("unsupported broker registry version {}", document.version);
        }
        if document.applications.len() > MAX_APPLICATIONS {
            bail!("broker application registry exceeds supported bounds");
        }
        let mut applications = BTreeMap::new();
        for authorization in document.applications {
            validate_identity(&authorization.identity)?;
            if applications
                .insert(authorization.identity.application_id.clone(), authorization)
                .is_some()
            {
                bail!("broker application registry contains a duplicate application id");
            }
        }
        if document.replay_proofs.len() > MAX_REPLAY_PROOFS {
            bail!("broker replay registry exceeds supported bounds");
        }
        let now = Utc::now().timestamp_millis();
        let replay_proofs = document
            .replay_proofs
            .into_iter()
            .filter(|proof| proof.expires_unix_ms > now)
            .map(|proof| {
                (
                    (proof.application_id, proof.nonce_sha256),
                    proof.expires_unix_ms,
                )
            })
            .collect();
        Ok(Self {
            root,
            applications,
            replay_proofs,
        })
    }

    /// Creates or rotates one app's capability. The plaintext token is
    /// returned once and never persisted.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid identities, capacity, or durable writes.
    pub fn authorize(
        &mut self,
        identity: ApplicationIdentity,
        capabilities: ApplicationCapabilities,
    ) -> Result<ApplicationAuthorization> {
        validate_identity(&identity)?;
        if !self.applications.contains_key(&identity.application_id)
            && self.applications.len() >= MAX_APPLICATIONS
        {
            bail!("broker application limit reached");
        }
        let token = generate_token();
        let mut applications = self.applications.clone();
        applications.insert(
            identity.application_id.clone(),
            DurableApplicationAuthorization {
                identity: identity.clone(),
                capabilities,
                token_sha256: token_digest(token.expose_for_client()),
                revoked: false,
            },
        );
        self.persist_applications(&applications)?;
        self.applications = applications;
        Ok(ApplicationAuthorization {
            identity,
            capabilities,
            token,
        })
    }

    #[must_use]
    pub fn authenticate(
        &self,
        application_id: &str,
        token: &str,
        requested: ApplicationCapabilities,
    ) -> bool {
        self.applications.get(application_id).is_some_and(|entry| {
            !entry.revoked
                && entry.capabilities.allows(requested)
                && constant_time_eq(
                    entry.token_sha256.as_bytes(),
                    token_digest(token).as_bytes(),
                )
        })
    }

    /// Verifies a protocol-v4 request and durably consumes its nonce before
    /// the authorized operation is dispatched. Returns the response proof key.
    ///
    /// # Errors
    ///
    /// Returns an error if the one-time replay reservation cannot be persisted.
    #[allow(clippy::too_many_arguments)]
    pub fn authenticate_proof(
        &mut self,
        request_id: uuid::Uuid,
        application_id: &str,
        capability: BrokerCapability,
        operation: &LocalOperation,
        nonce: &str,
        issued_unix_ms: i64,
        proof: &str,
        now_unix_ms: i64,
    ) -> Result<Option<[u8; 32]>> {
        if application_id.is_empty()
            || nonce.len() < 32
            || nonce.len() > 128
            || now_unix_ms.abs_diff(issued_unix_ms) > BROKER_PROOF_MAX_SKEW_MS.unsigned_abs()
        {
            return Ok(None);
        }
        let Some(entry) = self.applications.get(application_id) else {
            return Ok(None);
        };
        if entry.revoked
            || !entry
                .capabilities
                .allows(ApplicationCapabilities::requiring(capability))
        {
            return Ok(None);
        }
        let key: [u8; 32] = match hex::decode(&entry.token_sha256)
            .ok()
            .and_then(|value| value.try_into().ok())
        {
            Some(key) => key,
            None => return Ok(None),
        };
        let expected = broker_request_proof(
            &key,
            request_id,
            application_id,
            capability,
            operation,
            nonce,
            issued_unix_ms,
        )?;
        if !constant_time_proof_eq(&expected, proof) {
            return Ok(None);
        }
        let nonce_sha256 = hex::encode(Sha256::digest(nonce.as_bytes()));
        let replay_key = (application_id.to_owned(), nonce_sha256);
        let mut replay_proofs = self.replay_proofs.clone();
        replay_proofs.retain(|_, expiry| *expiry > now_unix_ms);
        if replay_proofs.contains_key(&replay_key) || replay_proofs.len() >= MAX_REPLAY_PROOFS {
            return Ok(None);
        }
        replay_proofs.insert(
            replay_key,
            issued_unix_ms.saturating_add(BROKER_PROOF_MAX_SKEW_MS),
        );
        self.persist_state(&self.applications, &replay_proofs)?;
        self.replay_proofs = replay_proofs;
        Ok(Some(key))
    }

    /// Revokes a capability durably before returning success.
    ///
    /// # Errors
    ///
    /// Returns an error when durable registry replacement fails.
    pub fn revoke(&mut self, application_id: &str) -> Result<bool> {
        let Some(entry) = self.applications.get(application_id) else {
            return Ok(false);
        };
        if entry.revoked {
            return Ok(false);
        }
        let mut applications = self.applications.clone();
        applications
            .get_mut(application_id)
            .context("broker authorization disappeared")?
            .revoked = true;
        self.persist_state(&applications, &self.replay_proofs)?;
        self.applications = applications;
        Ok(true)
    }

    fn persist_applications(
        &self,
        applications: &BTreeMap<String, DurableApplicationAuthorization>,
    ) -> Result<()> {
        self.persist_state(applications, &self.replay_proofs)
    }

    fn persist_state(
        &self,
        applications: &BTreeMap<String, DurableApplicationAuthorization>,
        replay_proofs: &BTreeMap<(String, String), i64>,
    ) -> Result<()> {
        let path = self.root.join("broker-applications.json");
        let document = BrokerDocument {
            version: DOCUMENT_VERSION,
            applications: applications.values().cloned().collect(),
            replay_proofs: replay_proofs
                .iter()
                .map(
                    |((application_id, nonce_sha256), expires_unix_ms)| DurableReplayProof {
                        application_id: application_id.clone(),
                        nonce_sha256: nonce_sha256.clone(),
                        expires_unix_ms: *expires_unix_ms,
                    },
                )
                .collect(),
        };
        let bytes = serde_json::to_vec_pretty(&document)?;
        crate::durable_file::replace_json(&path, &bytes)
    }
}

impl ApplicationCapabilities {
    const fn requiring(capability: BrokerCapability) -> Self {
        let mut requested = Self {
            observe_status: false,
            observe_printers: false,
            manage_profiles: false,
            submit_local_jobs: false,
            manage_connectors: false,
        };
        match capability {
            BrokerCapability::ObserveStatus => requested.observe_status = true,
            BrokerCapability::ObservePrinters => requested.observe_printers = true,
            BrokerCapability::ManageProfiles => requested.manage_profiles = true,
            BrokerCapability::SubmitLocalJobs => requested.submit_local_jobs = true,
            BrokerCapability::ManageConnectors => requested.manage_connectors = true,
        }
        requested
    }

    fn from_capabilities(capabilities: &[BrokerCapability]) -> Self {
        capabilities.iter().fold(
            Self {
                observe_status: false,
                observe_printers: false,
                manage_profiles: false,
                submit_local_jobs: false,
                manage_connectors: false,
            },
            |mut result, capability| {
                let requested = Self::requiring(*capability);
                result.observe_status |= requested.observe_status;
                result.observe_printers |= requested.observe_printers;
                result.manage_profiles |= requested.manage_profiles;
                result.submit_local_jobs |= requested.submit_local_jobs;
                result.manage_connectors |= requested.manage_connectors;
                result
            },
        )
    }
}

#[derive(Debug)]
struct PendingAuthorization {
    view: PendingBrokerAuthorization,
    nonce_sha256: String,
    decision: Option<Result<Vec<BrokerCapability>, ()>>,
}

#[derive(Debug, Default)]
struct ConsentState {
    pending: BTreeMap<uuid::Uuid, PendingAuthorization>,
}

#[derive(Clone)]
pub struct BrokerConsentHandle {
    registry: std::sync::Arc<Mutex<BrokerRegistry>>,
    consent: std::sync::Arc<Mutex<ConsentState>>,
}

impl std::fmt::Debug for BrokerConsentHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BrokerConsentHandle(<redacted>)")
    }
}

impl BrokerConsentHandle {
    pub async fn pending(&self) -> Vec<PendingBrokerAuthorization> {
        let now = Utc::now().timestamp_millis();
        let mut state = self.consent.lock().await;
        prune_expired(&mut state, now);
        state
            .pending
            .values()
            .filter(|pending| pending.decision.is_none())
            .map(|pending| pending.view.clone())
            .collect()
    }

    /// Applies an operator decision. Granted capabilities must be a subset of
    /// the application's request; claimed identity evidence is never trusted.
    ///
    /// # Errors
    ///
    /// Returns a bounded command failure when the request is absent, expired,
    /// already decided, or the granted set is not a subset of the request.
    #[allow(
        clippy::significant_drop_tightening,
        reason = "one consent lock makes decision validation and mutation atomic"
    )]
    pub async fn decide(
        &self,
        authorization_id: uuid::Uuid,
        decision: BrokerAuthorizationDecision,
    ) -> Result<(), CommandFailure> {
        let now = Utc::now().timestamp_millis();
        let mut state = self.consent.lock().await;
        prune_expired(&mut state, now);
        let pending = state
            .pending
            .get_mut(&authorization_id)
            .ok_or_else(|| CommandFailure {
                code: "broker_authorization_not_found".into(),
                message: "the authorization request was not found or expired".into(),
            })?;
        if pending.decision.is_some() {
            return Err(CommandFailure {
                code: "broker_authorization_already_decided".into(),
                message: "the authorization request has already been decided".into(),
            });
        }
        if !decision.approved {
            if !decision.granted_capabilities.is_empty() {
                return Err(CommandFailure {
                    code: "broker_authorization_invalid_decision".into(),
                    message: "a denied request cannot grant capabilities".into(),
                });
            }
            pending.decision = Some(Err(()));
            return Ok(());
        }
        let granted = validated_capabilities(&decision.granted_capabilities).map_err(|()| {
            CommandFailure {
                code: "broker_authorization_invalid_capabilities".into(),
                message: "approved capabilities must be a non-empty subset of the request".into(),
            }
        })?;
        let requested = pending
            .view
            .requested_capabilities
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if !granted
            .iter()
            .all(|capability| requested.contains(capability))
        {
            return Err(CommandFailure {
                code: "broker_authorization_invalid_capabilities".into(),
                message: "approved capabilities must be a non-empty subset of the request".into(),
            });
        }
        pending.decision = Some(Ok(granted));
        Ok(())
    }

    async fn request(
        &self,
        application: BrokerApplicationIdentity,
        capabilities: Vec<BrokerCapability>,
    ) -> Result<BrokerAuthorizationHandle, LocalFailure> {
        validate_identity(&application).map_err(|_| {
            local_failure(
                "invalid_application_identity",
                "the application identity is invalid",
                false,
            )
        })?;
        let capabilities = validated_capabilities(&capabilities).map_err(|()| {
            local_failure(
                "invalid_requested_capabilities",
                "at least one unique supported capability is required",
                false,
            )
        })?;
        let now = Utc::now().timestamp_millis();
        let mut state = self.consent.lock().await;
        prune_expired(&mut state, now);
        if state.pending.len() >= MAX_PENDING_AUTHORIZATIONS {
            return Err(local_failure(
                "authorization_capacity_reached",
                "the node has too many pending authorization requests",
                true,
            ));
        }
        let authorization_id = uuid::Uuid::new_v4();
        let nonce = generate_token();
        let expires_unix_ms = now.saturating_add(AUTHORIZATION_LIFETIME_MS);
        state.pending.insert(
            authorization_id,
            PendingAuthorization {
                view: PendingBrokerAuthorization {
                    authorization_id,
                    application,
                    requested_capabilities: capabilities,
                    requested_unix_ms: now,
                    expires_unix_ms,
                },
                nonce_sha256: token_digest(nonce.expose_for_client()),
                decision: None,
            },
        );
        let handle = BrokerAuthorizationHandle {
            authorization_id,
            nonce: nonce.expose_for_client().to_owned(),
            expires_unix_ms,
        };
        drop(state);
        Ok(handle)
    }

    #[allow(
        clippy::significant_drop_tightening,
        reason = "the authenticated consent state is read under one bounded lock"
    )]
    async fn status(
        &self,
        handle: &BrokerAuthorizationHandle,
    ) -> Result<BrokerAuthorizationState, LocalFailure> {
        let now = Utc::now().timestamp_millis();
        if handle.expires_unix_ms <= now {
            return Ok(BrokerAuthorizationState::Expired);
        }
        let mut state = self.consent.lock().await;
        prune_expired(&mut state, now);
        let pending = authenticated_pending(&state, handle)?;
        Ok(match &pending.decision {
            None => BrokerAuthorizationState::Pending,
            Some(Ok(_)) => BrokerAuthorizationState::Approved,
            Some(Err(())) => BrokerAuthorizationState::Denied,
        })
    }

    async fn exchange(
        &self,
        handle: &BrokerAuthorizationHandle,
    ) -> Result<BrokerCredential, LocalFailure> {
        let now = Utc::now().timestamp_millis();
        let mut state = self.consent.lock().await;
        prune_expired(&mut state, now);
        let pending = authenticated_pending(&state, handle)?;
        let capabilities = match &pending.decision {
            None => {
                return Err(local_failure(
                    "authorization_pending",
                    "the authorization request is awaiting a node-side decision",
                    true,
                ));
            }
            Some(Err(())) => {
                return Err(local_failure(
                    "authorization_denied",
                    "the node operator denied the authorization request",
                    false,
                ));
            }
            Some(Ok(capabilities)) => capabilities.clone(),
        };
        let application = pending.view.application.clone();
        let issued = self
            .registry
            .lock()
            .await
            .authorize(
                application.clone(),
                ApplicationCapabilities::from_capabilities(&capabilities),
            )
            .map_err(|_| {
                local_failure(
                    "authorization_persistence_failed",
                    "the approved capability could not be persisted",
                    true,
                )
            })?;
        state.pending.remove(&handle.authorization_id);
        let credential = BrokerCredential {
            application_id: application.application_id,
            token: issued.token.expose_for_client().to_owned(),
        };
        drop(state);
        Ok(credential)
    }
}

fn validated_capabilities(capabilities: &[BrokerCapability]) -> Result<Vec<BrokerCapability>, ()> {
    let unique = capabilities.iter().copied().collect::<BTreeSet<_>>();
    if unique.is_empty() || unique.len() != capabilities.len() || unique.len() > 5 {
        return Err(());
    }
    Ok(unique.into_iter().collect())
}

fn authenticated_pending<'a>(
    state: &'a ConsentState,
    handle: &BrokerAuthorizationHandle,
) -> Result<&'a PendingAuthorization, LocalFailure> {
    let pending = state.pending.get(&handle.authorization_id).ok_or_else(|| {
        local_failure(
            "authorization_not_found",
            "the authorization request was not found or has expired",
            false,
        )
    })?;
    if handle.expires_unix_ms != pending.view.expires_unix_ms
        || !constant_time_eq(
            pending.nonce_sha256.as_bytes(),
            token_digest(&handle.nonce).as_bytes(),
        )
    {
        return Err(local_failure(
            "authorization_invalid_nonce",
            "the authorization exchange secret is invalid",
            false,
        ));
    }
    Ok(pending)
}

fn prune_expired(state: &mut ConsentState, now: i64) {
    state
        .pending
        .retain(|_, pending| pending.view.expires_unix_ms > now);
}

#[derive(Clone)]
pub struct BrokerServerState {
    registry: std::sync::Arc<Mutex<BrokerRegistry>>,
    consent: std::sync::Arc<Mutex<ConsentState>>,
    commands: mpsc::Sender<RuntimeCommand>,
}

impl std::fmt::Debug for BrokerServerState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrokerServerState")
            .field("registry", &"<redacted>")
            .field("consent", &"<redacted>")
            .field("commands", &self.commands)
            .finish()
    }
}

impl BrokerServerState {
    #[must_use]
    pub fn new(registry: BrokerRegistry, commands: mpsc::Sender<RuntimeCommand>) -> Self {
        Self {
            registry: std::sync::Arc::new(Mutex::new(registry)),
            consent: std::sync::Arc::new(Mutex::new(ConsentState::default())),
            commands,
        }
    }

    #[must_use]
    pub fn consent_handle(&self) -> BrokerConsentHandle {
        BrokerConsentHandle {
            registry: std::sync::Arc::clone(&self.registry),
            consent: std::sync::Arc::clone(&self.consent),
        }
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the exhaustive protocol-version dispatch is intentionally kept at one boundary"
    )]
    async fn handle(&self, request: BrokerRequest) -> BrokerResponse {
        let mut response_authentication = None;
        let result = if (BROKER_PROTOCOL_MIN_VERSION..=BROKER_PROTOCOL_VERSION)
            .contains(&request.protocol)
        {
            match request.operation {
                BrokerOperation::Presence => Ok(BrokerResult::Presence(BrokerPresence {
                    protocol_min: BROKER_PROTOCOL_MIN_VERSION,
                    protocol_max: BROKER_PROTOCOL_VERSION,
                })),
                BrokerOperation::RequestAuthorization {
                    application,
                    requested_capabilities,
                } if request.protocol >= 2 => self
                    .consent_handle()
                    .request(application, requested_capabilities)
                    .await
                    .map(BrokerResult::AuthorizationRequested),
                BrokerOperation::AuthorizationStatus { handle } if request.protocol >= 2 => self
                    .consent_handle()
                    .status(&handle)
                    .await
                    .map(|state| BrokerResult::AuthorizationStatus { state }),
                BrokerOperation::ExchangeAuthorization { handle } if request.protocol >= 2 => self
                    .consent_handle()
                    .exchange(&handle)
                    .await
                    .map(BrokerResult::AuthorizationExchanged),
                BrokerOperation::RequestAuthorization { .. }
                | BrokerOperation::AuthorizationStatus { .. }
                | BrokerOperation::ExchangeAuthorization { .. } => Err(local_failure(
                    "unsupported_broker_protocol",
                    "authorization consent requires broker protocol version 2",
                    false,
                )),
                BrokerOperation::Execute { operation, .. } => {
                    let _ = operation;
                    Err(local_failure(
                        "broker_protocol_upgrade_required",
                        "secret-bearing broker execution requires protocol version 4",
                        false,
                    ))
                }
                BrokerOperation::ExecuteAuthenticated {
                    application_id,
                    capability,
                    operation,
                    nonce,
                    issued_unix_ms,
                    proof,
                } if request.protocol == 4 => {
                    let required = required_capability(&operation);
                    if required == Some(capability) {
                        let authentication = self.registry.lock().await.authenticate_proof(
                            request.request_id,
                            &application_id,
                            capability,
                            &operation,
                            &nonce,
                            issued_unix_ms,
                            &proof,
                            Utc::now().timestamp_millis(),
                        );
                        match authentication {
                            Ok(Some(key)) => {
                                response_authentication = Some((key, nonce));
                                dispatch_operation(&self.commands, operation)
                                    .await
                                    .map(|result| BrokerResult::Local { result })
                            }
                            Ok(None) => Err(local_failure(
                                "application_unauthorized",
                                "the application proof is invalid, stale, replayed, or revoked",
                                false,
                            )),
                            Err(_) => Err(local_failure(
                                "broker_state_unavailable",
                                "the broker could not durably reserve this request",
                                true,
                            )),
                        }
                    } else {
                        Err(local_failure(
                            "capability_mismatch",
                            "the declared capability does not authorize this operation",
                            false,
                        ))
                    }
                }
                BrokerOperation::ExecuteAuthenticated { .. } => Err(local_failure(
                    "unsupported_broker_protocol",
                    "authenticated execution requires broker protocol version 4",
                    false,
                )),
            }
        } else {
            Err(local_failure(
                "unsupported_broker_protocol",
                "the application and node broker protocol versions do not overlap",
                false,
            ))
        };
        let proof = response_authentication.and_then(|(key, nonce)| {
            broker_response_proof(&key, request.request_id, &nonce, &result).ok()
        });
        BrokerResponse {
            protocol: BROKER_PROTOCOL_VERSION,
            request_id: request.request_id,
            result,
            proof,
        }
    }
}

#[cfg(unix)]
/// Serves the application broker on a private Unix-domain socket.
///
/// # Errors
///
/// Returns an error if the endpoint cannot be safely bound or accepted.
pub async fn serve_unix_broker(
    path: impl Into<PathBuf>,
    state: BrokerServerState,
) -> Result<(), piqae_local_ipc::LocalIpcError> {
    let endpoint = piqae_local_ipc::LocalEndpoint::bind(path)?;
    let capacity = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_BROKER_CONNECTIONS));
    loop {
        let mut stream = endpoint.accept().await?;
        let Ok(permit) = std::sync::Arc::clone(&capacity).try_acquire_owned() else {
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _permit = permit;
            serve_connection(&mut stream, state).await;
        });
    }
}

#[cfg(windows)]
/// Serves the broker on a remote-client-rejecting, current-user-only named
/// pipe. Creation fails closed if the pipe name is already squatted or its ACL
/// cannot be established.
///
/// # Errors
///
/// Returns a local IPC error when the protected endpoint cannot be created or
/// a pipe instance cannot accept a client.
pub async fn serve_windows_broker(
    name: &str,
    state: BrokerServerState,
) -> Result<(), piqae_local_ipc::LocalIpcError> {
    let mut server = piqae_local_ipc::create_current_user_pipe_server(name, true)?;
    let capacity = std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_BROKER_CONNECTIONS));
    loop {
        server.connect().await?;
        let mut connected = server;
        server = piqae_local_ipc::create_current_user_pipe_server(name, false)?;
        let Ok(permit) = std::sync::Arc::clone(&capacity).try_acquire_owned() else {
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _permit = permit;
            serve_connection(&mut connected, state).await;
        });
    }
}

async fn serve_connection(
    stream: &mut (impl tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send),
    state: BrokerServerState,
) {
    let Ok(Ok(request)) =
        tokio::time::timeout(BROKER_IO_TIMEOUT, read_message::<BrokerRequest>(stream)).await
    else {
        return;
    };
    let response = state.handle(request).await;
    let _ = tokio::time::timeout(BROKER_IO_TIMEOUT, write_message(stream, &response)).await;
}

const fn required_capability(operation: &LocalOperation) -> Option<BrokerCapability> {
    match operation {
        LocalOperation::Status => Some(BrokerCapability::ObserveStatus),
        LocalOperation::Printers => Some(BrokerCapability::ObservePrinters),
        LocalOperation::BeginProfileCapture(_)
        | LocalOperation::CommitProfileCapture(_)
        | LocalOperation::CancelProfileCapture(_)
        | LocalOperation::ValidateProfile(_)
        | LocalOperation::ConfirmLoadedMedia(_) => Some(BrokerCapability::ManageProfiles),
        LocalOperation::Pause | LocalOperation::Resume => Some(BrokerCapability::ManageConnectors),
        LocalOperation::Sdk { operation } => Some(match operation {
            SdkBrokerOperation::ConnectInvitation { .. }
            | SdkBrokerOperation::RevokeConnector { .. } => BrokerCapability::ManageConnectors,
            SdkBrokerOperation::SubmitLocalJob { .. } => BrokerCapability::SubmitLocalJobs,
            SdkBrokerOperation::Profiles { .. } => BrokerCapability::ObservePrinters,
            SdkBrokerOperation::JobHistory { .. } | SdkBrokerOperation::ConnectorSnapshots => {
                BrokerCapability::ObserveStatus
            }
        }),
        LocalOperation::RestartAgent
        | LocalOperation::ExportSupportBundle { .. }
        | LocalOperation::Reenrol { .. } => None,
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "exhaustive protocol-to-command mapping stays in one auditable boundary"
)]
async fn dispatch_operation(
    commands: &mpsc::Sender<RuntimeCommand>,
    operation: LocalOperation,
) -> Result<LocalResult, LocalFailure> {
    match operation {
        LocalOperation::Status => {
            let (send, receive) = oneshot::channel();
            send_command(commands, RuntimeCommand::Status { respond_to: send }).await?;
            receive
                .await
                .map(LocalResult::Status)
                .map_err(|_| unavailable())
        }
        LocalOperation::Printers => {
            let (send, receive) = oneshot::channel();
            send_command(commands, RuntimeCommand::Printers { respond_to: send }).await?;
            receive
                .await
                .map(|printers| LocalResult::Printers { printers })
                .map_err(|_| unavailable())
        }
        LocalOperation::Sdk { operation } => dispatch_sdk_operation(commands, operation).await,
        LocalOperation::Pause | LocalOperation::Resume => {
            let (send, receive) = oneshot::channel();
            let command = if matches!(operation, LocalOperation::Pause) {
                RuntimeCommand::Pause { respond_to: send }
            } else {
                RuntimeCommand::Resume { respond_to: send }
            };
            send_command(commands, command).await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(|()| LocalResult::Accepted)
                .map_err(command_failure)
        }
        LocalOperation::BeginProfileCapture(request) => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::BeginProfileCapture {
                    printer_id: request.printer_id.clone(),
                    request: crate::command::ProfileCaptureBeginRequest {
                        operation: request.operation,
                        profile_id: request.profile_id,
                        expected_revision: request.expected_revision,
                    },
                    respond_to: send,
                },
            )
            .await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(|authorization| LocalResult::ProfileCaptureAuthorized(Box::new(authorization)))
                .map_err(command_failure)
        }
        LocalOperation::CommitProfileCapture(request) => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::CommitProfileCapture {
                    session_id: request.session_id,
                    capture_token: request.capture_token,
                    capture: Box::new(request.capture),
                    respond_to: send,
                },
            )
            .await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(|profile| LocalResult::ProfileCaptured {
                    profile: Box::new(profile),
                })
                .map_err(command_failure)
        }
        LocalOperation::CancelProfileCapture(request) => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::CancelProfileCapture {
                    session_id: request.session_id,
                    capture_token: request.capture_token,
                    respond_to: send,
                },
            )
            .await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(|()| LocalResult::Accepted)
                .map_err(command_failure)
        }
        LocalOperation::ValidateProfile(request) => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::ValidateProfile {
                    profile_id: request.profile_id,
                    revision: request.revision,
                    respond_to: send,
                },
            )
            .await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(LocalResult::ProfileValidation)
                .map_err(command_failure)
        }
        LocalOperation::ConfirmLoadedMedia(request) => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::ConfirmLoadedMedia {
                    request,
                    respond_to: send,
                },
            )
            .await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map(|()| LocalResult::Accepted)
                .map_err(command_failure)
        }
        LocalOperation::RestartAgent
        | LocalOperation::ExportSupportBundle { .. }
        | LocalOperation::Reenrol { .. } => Err(local_failure(
            "operation_requires_native_shell",
            "this privileged operation is not available to application clients",
            false,
        )),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one exhaustive SDK wire-to-command mapping keeps capability auditability"
)]
async fn dispatch_sdk_operation(
    commands: &mpsc::Sender<RuntimeCommand>,
    operation: SdkBrokerOperation,
) -> Result<LocalResult, LocalFailure> {
    let data = match operation {
        SdkBrokerOperation::ConnectInvitation {
            control_plane_url,
            invitation_token,
            printer_grant,
            allowed_printer_ids,
            node_name,
            hostname,
        } => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::ConnectInvitation {
                    request: Box::new(crate::command::ConnectorInvitationRequest {
                        control_plane_url,
                        invitation_token: invitation_token.expose_for_exchange(),
                        printer_grant,
                        allowed_printer_ids,
                        node_name,
                        hostname,
                    }),
                    respond_to: send,
                },
            )
            .await?;
            serde_json::to_value(
                receive
                    .await
                    .map_err(|_| unavailable())?
                    .map_err(command_failure)?,
            )
        }
        SdkBrokerOperation::SubmitLocalJob {
            printer_id,
            title,
            content_kind,
            content_base64,
            options,
            expires_unix_ms,
        } => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::SubmitJob {
                    request: Box::new(crate::command::LocalCreateJob {
                        printer_id,
                        printer_native_id: None,
                        title,
                        content_kind,
                        content: crate::command::LocalContent::Base64 {
                            data: content_base64,
                        },
                        options,
                        expires_unix_ms,
                    }),
                    respond_to: send,
                },
            )
            .await?;
            serde_json::to_value(
                receive
                    .await
                    .map_err(|_| unavailable())?
                    .map_err(command_failure)?,
            )
        }
        SdkBrokerOperation::Profiles { printer_id } => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::Profiles {
                    printer_id,
                    respond_to: send,
                },
            )
            .await?;
            serde_json::to_value(
                receive
                    .await
                    .map_err(|_| unavailable())?
                    .map_err(command_failure)?,
            )
        }
        SdkBrokerOperation::JobHistory { offset, limit } => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::JobHistory {
                    offset,
                    limit,
                    respond_to: send,
                },
            )
            .await?;
            serde_json::to_value(
                receive
                    .await
                    .map_err(|_| unavailable())?
                    .map_err(command_failure)?,
            )
        }
        SdkBrokerOperation::ConnectorSnapshots => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::ConnectorDetails { respond_to: send },
            )
            .await?;
            serde_json::to_value(
                receive
                    .await
                    .map_err(|_| unavailable())?
                    .map_err(command_failure)?,
            )
        }
        SdkBrokerOperation::RevokeConnector { connector_id } => {
            let (send, receive) = oneshot::channel();
            send_command(
                commands,
                RuntimeCommand::RevokeConnector {
                    connector_id,
                    respond_to: send,
                },
            )
            .await?;
            receive
                .await
                .map_err(|_| unavailable())?
                .map_err(command_failure)?;
            Ok(serde_json::json!({ "revoked": true }))
        }
    }
    .map_err(|_| {
        local_failure(
            "sdk_result_serialization_failed",
            "the local SDK result could not be serialized",
            false,
        )
    })?;
    Ok(LocalResult::Sdk { data })
}

async fn send_command(
    commands: &mpsc::Sender<RuntimeCommand>,
    command: RuntimeCommand,
) -> Result<(), LocalFailure> {
    commands.send(command).await.map_err(|_| unavailable())
}

fn unavailable() -> LocalFailure {
    local_failure(
        "node_runtime_unavailable",
        "the durable node runtime is unavailable",
        true,
    )
}

fn command_failure(CommandFailure { code, message }: CommandFailure) -> LocalFailure {
    local_failure(&code, &message, false)
}

fn local_failure(code: &str, message: &str, retryable: bool) -> LocalFailure {
    LocalFailure {
        code: code.to_owned(),
        message: message.to_owned(),
        retryable,
    }
}

fn validate_identity(identity: &ApplicationIdentity) -> Result<()> {
    if identity.application_id.is_empty()
        || identity.application_id.len() > 255
        || !identity
            .application_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        || identity.display_name.is_empty()
        || identity.display_name.len() > 128
        || identity
            .signing_identity_sha256
            .as_ref()
            .is_some_and(|value| {
                value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
    {
        bail!("invalid broker application identity");
    }
    Ok(())
}

fn generate_token() -> BrokerToken {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    BrokerToken(URL_SAFE_NO_PAD.encode(bytes))
}

fn token_digest(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use piqae_local_ipc::{
        BrokerOperation, BrokerRequest, ConnectionState, broker_proof_key, broker_request_proof,
    };
    use uuid::Uuid;

    fn identity() -> ApplicationIdentity {
        ApplicationIdentity {
            application_id: "com.example.pos".into(),
            display_name: "Example POS".into(),
            signing_identity_sha256: None,
        }
    }

    #[test]
    fn token_is_returned_once_and_registry_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = BrokerRegistry::load(directory.path()).unwrap();
        let issued = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        assert!(!format!("{issued:?}").contains(issued.token.expose_for_client()));
        drop(registry);

        let registry = BrokerRegistry::load(directory.path()).unwrap();
        assert!(registry.authenticate(
            "com.example.pos",
            issued.token.expose_for_client(),
            ApplicationCapabilities::OBSERVE_ONLY
        ));
    }

    #[test]
    fn least_privilege_and_revocation_are_enforced() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = BrokerRegistry::load(directory.path()).unwrap();
        let issued = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        assert!(!registry.authenticate(
            "com.example.pos",
            issued.token.expose_for_client(),
            ApplicationCapabilities {
                submit_local_jobs: true,
                ..ApplicationCapabilities::OBSERVE_ONLY
            }
        ));
        assert!(registry.revoke("com.example.pos").unwrap());
        assert!(!registry.authenticate(
            "com.example.pos",
            issued.token.expose_for_client(),
            ApplicationCapabilities::OBSERVE_ONLY
        ));
    }

    #[test]
    fn rotating_one_app_does_not_authorize_the_previous_token() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = BrokerRegistry::load(directory.path()).unwrap();
        let old = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        let current = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        assert!(!registry.authenticate(
            "com.example.pos",
            old.token.expose_for_client(),
            ApplicationCapabilities::OBSERVE_ONLY
        ));
        assert!(registry.authenticate(
            "com.example.pos",
            current.token.expose_for_client(),
            ApplicationCapabilities::OBSERVE_ONLY
        ));
    }

    #[test]
    fn repeated_authorize_rotate_revoke_is_durable_across_restart() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = BrokerRegistry::load(directory.path()).unwrap();
        let first = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        let second = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        assert!(!registry.authenticate(
            "com.example.pos",
            first.token.expose_for_client(),
            ApplicationCapabilities::OBSERVE_ONLY
        ));
        assert!(registry.revoke("com.example.pos").unwrap());
        drop(registry);

        let registry = BrokerRegistry::load(directory.path()).unwrap();
        assert!(!registry.authenticate(
            "com.example.pos",
            first.token.expose_for_client(),
            ApplicationCapabilities::OBSERVE_ONLY
        ));
        assert!(!registry.authenticate(
            "com.example.pos",
            second.token.expose_for_client(),
            ApplicationCapabilities::OBSERVE_ONLY
        ));
    }

    #[tokio::test]
    async fn broker_dispatches_only_an_authorized_capability() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = BrokerRegistry::load(directory.path()).unwrap();
        let issued = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        let (commands, mut receive) = mpsc::channel(1);
        let state = BrokerServerState::new(registry, commands);
        tokio::spawn(async move {
            if let Some(RuntimeCommand::Status { respond_to }) = receive.recv().await {
                let _ = respond_to.send(piqae_local_ipc::LocalStatus {
                    agent_id: None,
                    workspace_name: None,
                    version: "test".into(),
                    connection: ConnectionState::LocalOnly,
                    queued_jobs: 0,
                    active_jobs: 0,
                    printer_warnings: 0,
                    paused: false,
                });
            }
        });
        let request_id = Uuid::new_v4();
        let nonce = Uuid::new_v4().simple().to_string();
        let issued_unix_ms = Utc::now().timestamp_millis();
        let proof = broker_request_proof(
            &broker_proof_key(issued.token.expose_for_client()),
            request_id,
            "com.example.pos",
            BrokerCapability::ObserveStatus,
            &LocalOperation::Status,
            &nonce,
            issued_unix_ms,
        )
        .unwrap();
        let request = BrokerRequest {
            protocol: BROKER_PROTOCOL_VERSION,
            request_id,
            operation: BrokerOperation::ExecuteAuthenticated {
                application_id: "com.example.pos".into(),
                capability: BrokerCapability::ObserveStatus,
                operation: LocalOperation::Status,
                nonce,
                issued_unix_ms,
                proof,
            },
        };
        let serialized = serde_json::to_string(&request).unwrap();
        assert!(!serialized.contains(issued.token.expose_for_client()));
        let response = state.handle(request).await;
        assert!(matches!(
            response.result,
            Ok(BrokerResult::Local {
                result: LocalResult::Status(_)
            })
        ));
        assert!(response.proof.is_some());
    }

    #[test]
    fn authenticated_proofs_reject_tamper_and_replay_across_restart() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = BrokerRegistry::load(directory.path()).unwrap();
        let issued = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        let key = broker_proof_key(issued.token.expose_for_client());
        let request_id = Uuid::new_v4();
        let nonce = Uuid::new_v4().simple().to_string();
        let now = Utc::now().timestamp_millis();
        let proof = broker_request_proof(
            &key,
            request_id,
            "com.example.pos",
            BrokerCapability::ObserveStatus,
            &LocalOperation::Status,
            &nonce,
            now,
        )
        .unwrap();
        assert!(
            registry
                .authenticate_proof(
                    request_id,
                    "com.example.pos",
                    BrokerCapability::ObserveStatus,
                    &LocalOperation::Status,
                    &nonce,
                    now,
                    &proof,
                    now,
                )
                .unwrap()
                .is_some()
        );
        drop(registry);

        let mut restarted = BrokerRegistry::load(directory.path()).unwrap();
        assert!(
            restarted
                .authenticate_proof(
                    request_id,
                    "com.example.pos",
                    BrokerCapability::ObserveStatus,
                    &LocalOperation::Status,
                    &nonce,
                    now,
                    &proof,
                    now,
                )
                .unwrap()
                .is_none()
        );
        assert!(
            restarted
                .authenticate_proof(
                    Uuid::new_v4(),
                    "com.example.pos",
                    BrokerCapability::ObserveStatus,
                    &LocalOperation::Printers,
                    &Uuid::new_v4().simple().to_string(),
                    now,
                    &proof,
                    now,
                )
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn raw_token_execute_is_rejected_after_protocol_upgrade() {
        let directory = tempfile::tempdir().unwrap();
        let mut registry = BrokerRegistry::load(directory.path()).unwrap();
        let issued = registry
            .authorize(identity(), ApplicationCapabilities::OBSERVE_ONLY)
            .unwrap();
        let (commands, _receive) = mpsc::channel(1);
        let response = BrokerServerState::new(registry, commands)
            .handle(BrokerRequest {
                protocol: 3,
                request_id: Uuid::new_v4(),
                operation: BrokerOperation::Execute {
                    credential: piqae_local_ipc::BrokerCredential {
                        application_id: "com.example.pos".into(),
                        token: issued.token.expose_for_client().into(),
                    },
                    capability: BrokerCapability::ObserveStatus,
                    operation: LocalOperation::Status,
                },
            })
            .await;
        assert!(matches!(
            response.result,
            Err(LocalFailure { ref code, .. }) if code == "broker_protocol_upgrade_required"
        ));
        assert!(response.proof.is_none());
    }

    #[tokio::test]
    async fn consent_requires_node_decision_and_exchange_is_one_time() {
        let directory = tempfile::tempdir().unwrap();
        let (commands, _receive) = mpsc::channel(1);
        let state =
            BrokerServerState::new(BrokerRegistry::load(directory.path()).unwrap(), commands);
        let consent = state.consent_handle();
        let handle = consent
            .request(
                identity(),
                vec![
                    BrokerCapability::ObserveStatus,
                    BrokerCapability::ObservePrinters,
                ],
            )
            .await
            .unwrap();
        assert!(!format!("{handle:?}").contains(&handle.nonce));
        assert_eq!(consent.pending().await.len(), 1);
        assert!(!state.registry.lock().await.authenticate(
            "com.example.pos",
            "claimed-signing-identity-is-not-a-token",
            ApplicationCapabilities::OBSERVE_ONLY,
        ));
        consent
            .decide(
                handle.authorization_id,
                BrokerAuthorizationDecision {
                    approved: true,
                    granted_capabilities: vec![BrokerCapability::ObserveStatus],
                },
            )
            .await
            .unwrap();
        assert_eq!(
            consent.status(&handle).await.unwrap(),
            BrokerAuthorizationState::Approved
        );
        let credential = consent.exchange(&handle).await.unwrap();
        assert!(!format!("{credential:?}").contains(&credential.token));
        assert!(matches!(
            consent.exchange(&handle).await,
            Err(LocalFailure { code, .. }) if code == "authorization_not_found"
        ));
    }

    #[tokio::test]
    async fn pending_consent_expires_and_never_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        let (commands, _receive) = mpsc::channel(1);
        let state =
            BrokerServerState::new(BrokerRegistry::load(directory.path()).unwrap(), commands);
        let consent = state.consent_handle();
        let handle = consent
            .request(identity(), vec![BrokerCapability::ObserveStatus])
            .await
            .unwrap();
        {
            let mut pending = consent.consent.lock().await;
            pending
                .pending
                .get_mut(&handle.authorization_id)
                .unwrap()
                .view
                .expires_unix_ms = Utc::now().timestamp_millis() - 1;
        }
        assert!(consent.pending().await.is_empty());

        let handle = consent
            .request(identity(), vec![BrokerCapability::ObserveStatus])
            .await
            .unwrap();
        drop(state);
        let (commands, _receive) = mpsc::channel(1);
        let restarted =
            BrokerServerState::new(BrokerRegistry::load(directory.path()).unwrap(), commands);
        assert!(matches!(
            restarted.consent_handle().status(&handle).await,
            Err(LocalFailure { code, .. }) if code == "authorization_not_found"
        ));
    }
}
