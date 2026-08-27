//! Identity-provider configuration matrix for the tenant-facing API.
//!
//! Piqae ships one binary that runs under several identity providers:
//! `local_owner` for self-hosted installs, and `workos` or generic OIDC for the
//! managed cloud. A handler that reaches for provider-specific state serves a
//! healthy `/v1/ready` and still fails every real request in the deployment it
//! was not exercised in, which is invisible to a suite that only ever builds
//! one configuration.
//!
//! These tests build the real router once per provider configuration and assert
//! the provider-independent contract in each of them.
//!
//! Limits, stated honestly:
//!
//! * The configurations here differ in exactly the way production differs: the
//!   deployment capabilities and whether local-owner state is attached. They do
//!   not run a real `WorkOS` or OIDC token verifier, so token-shape regressions
//!   are out of scope; `authentication.rs` owns those.
//! * `AppState::with_local_identity` needs a live `PostgresStore`, so the
//!   local-owner-attached variant cannot be built in-process. The routes that
//!   depend on it are asserted to fail closed instead, and the database-backed
//!   suites cover the attached path.
//! * Coverage of the route list is enforced from the router source rather than
//!   curated by hand, so a new identity route cannot be added without either
//!   entering this matrix or being declared local-owner-only.

#![allow(clippy::expect_used)]

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::Utc;
use http_body_util::BodyExt;
use piqae_control_plane::{
    AppState, AuthCapabilities, DeploymentCapabilities,
    authentication::{StaticAuthenticator, TenantContext},
    repository::MemoryRepository,
    router,
};
use piqae_domain::{AgentId, EnvironmentId, WorkspaceId};
use piqae_storage_postgres::StoredWorkspaceMember;
use std::sync::Arc;
use tower::ServiceExt;

/// Every identity route below this prefix is local-owner-only by design and
/// must fail closed under any other provider.
const LOCAL_OWNER_ONLY_PREFIX: &str = "/v1/identity/local/";

/// Routes that the published contract serves in every deployment, with the
/// methods `contracts/openapi/piqae-v1.yaml` documents for them.
const PROVIDER_INDEPENDENT: &[(&str, &str)] = &[
    ("GET", "/v1/identity/me"),
    ("GET", "/v1/workspaces/current"),
    ("PATCH", "/v1/workspaces/current"),
    ("GET", "/v1/workspaces/current/members"),
];

/// The identity-provider values a deployment can be configured with.
const PROVIDERS: &[&str] = &["local_owner", "workos", "oidc"];

const TOKEN: &str = "matrix-tenant-token";

/// Builds the API exactly as a deployment configured for `provider` builds it.
///
/// `local_identity` is left unattached, which is what every provider other than
/// `local_owner` produces in `main.rs`, and what production runs.
async fn deployment(provider: &str, member_role: &str) -> axum::Router {
    let repository = MemoryRepository::default();
    let workspace_id = WorkspaceId::new();
    let environment_id = EnvironmentId::new();
    let now = Utc::now();
    repository
        .add_workspace(
            workspace_id,
            "Matrix workspace",
            vec![StoredWorkspaceMember {
                id: "usr_matrix".into(),
                email: "owner@example.test".into(),
                name: Some("Matrix Owner".into()),
                role: member_role.into(),
                status: "active".into(),
                created_at: now,
                updated_at: now,
            }],
        )
        .await;

    let authenticator = StaticAuthenticator::default();
    authenticator
        .insert(
            TOKEN,
            TenantContext::unrestricted(workspace_id, environment_id),
        )
        .await;

    let cloud = provider != "local_owner";
    let mut capabilities = DeploymentCapabilities {
        deployment: if cloud {
            "cloud".into()
        } else {
            "self_hosted".into()
        },
        auth: AuthCapabilities {
            provider: provider.to_owned(),
            workspace_switching: cloud,
            invitations: cloud,
        },
        ..DeploymentCapabilities::default()
    };
    capabilities.billing.enabled = cloud;

    router(
        AppState::new_for_tests(Arc::new(repository), Arc::new(authenticator))
            .with_capabilities(capabilities),
    )
}

fn request(method: &str, path: &str, bearer: Option<&str>, body: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(bearer) = bearer {
        builder = builder.header("authorization", format!("Bearer {bearer}"));
    }
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    builder
        .body(body.map_or_else(Body::empty, |value| Body::from(value.to_owned())))
        .expect("HTTP request")
}

/// A structurally valid body for a route, so a rejected request is rejected on
/// identity rather than on deserialization. A `422` from the `Json` extractor
/// would mask the availability answer these tests are asking for.
fn body_for(path: &str) -> Option<&'static str> {
    match path {
        "/v1/workspaces/current" => Some(r#"{"name":"Renamed workspace"}"#),
        "/v1/identity/local/bootstrap" => {
            Some(r#"{"workspace_name":"Matrix","email":"owner@example.test"}"#)
        }
        "/v1/identity/local/exchange" => Some(r#"{"credential":"lo_not_a_real_credential"}"#),
        _ => None,
    }
}

async fn json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body")
        .to_bytes();
    serde_json::from_slice(&bytes).expect("JSON body")
}

/// The route table is read from the router source so this matrix cannot fall
/// behind it: a new identity route must be listed here or be local-owner-only.
fn declared_identity_routes() -> Vec<String> {
    let source = include_str!("../src/identity.rs");
    let mut routes = Vec::new();
    for fragment in source.split(".route(").skip(1) {
        let Some(rest) = fragment.trim_start().strip_prefix('"') else {
            continue;
        };
        let Some(end) = rest.find('"') else { continue };
        let path = &rest[..end];
        if path.starts_with("/v1/") && !routes.iter().any(|known| known == path) {
            routes.push(path.to_owned());
        }
    }
    routes
}

#[test]
fn the_matrix_covers_every_provider_independent_identity_route() {
    let declared = declared_identity_routes();
    assert!(
        declared.len() >= PROVIDER_INDEPENDENT.len(),
        "no identity routes were parsed from the router source: {declared:?}"
    );
    for path in &declared {
        if path.starts_with(LOCAL_OWNER_ONLY_PREFIX) {
            continue;
        }
        assert!(
            PROVIDER_INDEPENDENT
                .iter()
                .any(|(_, covered)| covered == path),
            "{path} is served in every deployment but is not exercised by the \
             identity-provider matrix; add it to PROVIDER_INDEPENDENT or move \
             it under {LOCAL_OWNER_ONLY_PREFIX}"
        );
    }
}

/// The production incident in one assertion.
///
/// `GET`/`PATCH /v1/workspaces/current` answered `503` under `workos` because
/// the handlers required local-owner state before authenticating. An
/// unauthenticated probe separates the two failures exactly: a route that is
/// merely unauthenticated answers `401`, a route that is structurally
/// unavailable answers `503`.
#[tokio::test]
async fn provider_independent_routes_are_never_unavailable_in_any_configuration() {
    for provider in PROVIDERS {
        for (method, path) in PROVIDER_INDEPENDENT {
            let deployment = deployment(provider, "owner").await;
            let response = deployment
                .oneshot(request(method, path, None, body_for(path)))
                .await
                .expect("router response");
            let status = response.status();
            assert_ne!(
                status,
                StatusCode::SERVICE_UNAVAILABLE,
                "{method} {path} is unavailable under identity provider \
                 '{provider}': {:?}",
                json(response).await
            );
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{method} {path} under '{provider}' must reject an \
                 unauthenticated caller on authentication alone"
            );
        }
    }
}

#[tokio::test]
async fn revisioned_node_identity_patch_is_available_under_every_provider() {
    for provider in PROVIDERS {
        let deployment = deployment(provider, "owner").await;
        let response = deployment
            .oneshot(request(
                "PATCH",
                &format!("/v1/nodes/{}", AgentId::new()),
                None,
                Some(
                    r#"{"name":"Dispatch node","site":null,"location":null,"labels":[],"expected_revision":1}"#,
                ),
            ))
            .await
            .expect("router response");
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "node identity PATCH must authenticate rather than become unavailable under {provider}"
        );
    }
}

#[tokio::test]
async fn workspace_endpoints_serve_an_authenticated_caller_under_every_provider() {
    for provider in PROVIDERS {
        let deployment = deployment(provider, "owner").await;

        let response = deployment
            .clone()
            .oneshot(request("GET", "/v1/workspaces/current", Some(TOKEN), None))
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::OK, "provider {provider}");
        let workspace = json(response).await;
        let workspace_id = workspace["id"].clone();
        assert!(workspace_id.is_string(), "provider {provider}");
        assert_eq!(workspace["name"], "Matrix workspace");

        let response = deployment
            .clone()
            .oneshot(request(
                "PATCH",
                "/v1/workspaces/current",
                Some(TOKEN),
                Some(r#"{"name":"Renamed workspace"}"#),
            ))
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::OK, "provider {provider}");
        assert_eq!(json(response).await["name"], "Renamed workspace");

        let response = deployment
            .clone()
            .oneshot(request(
                "GET",
                "/v1/workspaces/current/members",
                Some(TOKEN),
                None,
            ))
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::OK, "provider {provider}");
        assert_eq!(json(response).await[0]["email"], "owner@example.test");

        let response = deployment
            .oneshot(request("GET", "/v1/identity/me", Some(TOKEN), None))
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::OK, "provider {provider}");
        let identity = json(response).await;
        assert_eq!(identity["workspace_id"], workspace_id);
        assert_eq!(identity["roles"][0], "owner");
    }
}

/// Decoupling the workspace endpoints from local-owner state must not have
/// widened who may rename a workspace.
#[tokio::test]
async fn a_plain_member_still_cannot_rename_the_workspace_under_any_provider() {
    for provider in PROVIDERS {
        let deployment = deployment(provider, "member").await;
        let response = deployment
            .oneshot(request(
                "PATCH",
                "/v1/workspaces/current",
                Some(TOKEN),
                Some(r#"{"name":"Hijacked"}"#),
            ))
            .await
            .expect("router response");
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "provider {provider}"
        );
    }
}

/// The local-owner credential routes must stay fail-closed when the deployment
/// runs another provider: they mint self-hosted owner credentials.
#[tokio::test]
async fn local_owner_credential_routes_fail_closed_without_local_identity() {
    for path in declared_identity_routes()
        .into_iter()
        .filter(|path| path.starts_with(LOCAL_OWNER_ONLY_PREFIX))
    {
        for provider in PROVIDERS {
            let deployment = deployment(provider, "owner").await;
            let response = deployment
                .oneshot(request("POST", &path, Some(TOKEN), body_for(&path)))
                .await
                .expect("router response");
            assert_eq!(
                response.status(),
                StatusCode::SERVICE_UNAVAILABLE,
                "{path} must fail closed without local-owner state (provider \
                 {provider})"
            );
            assert_eq!(
                json(response).await["error"]["code"],
                "local_identity_disabled"
            );
        }
    }
}

/// `/v1/meta` is what the dashboard branches on, so the configuration under
/// test must actually be visible to a client.
#[tokio::test]
async fn deployment_capabilities_report_the_configured_provider() {
    for provider in PROVIDERS {
        let deployment = deployment(provider, "owner").await;
        let response = deployment
            .oneshot(request("GET", "/v1/meta", None, None))
            .await
            .expect("router response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json(response).await["auth"]["provider"], *provider);
    }
}

/// Ties the post-deploy gate to the real router.
///
/// `release/post-deploy-probes.json` drives an unauthenticated smoke check
/// against live deployments, and its whole value rests on one assumption: a
/// healthy tenant endpoint answers `401` without credentials. Proving that
/// in-process, under every identity provider, is what stops the deploy gate
/// from being either a false alarm or a rubber stamp.
#[tokio::test]
async fn the_post_deploy_probe_manifest_matches_the_real_router() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../../release/post-deploy-probes.json"))
            .expect("probe manifest");
    let probes = manifest["services"]["piqae-control-plane"]["probes"]
        .as_array()
        .expect("control-plane probes");
    assert!(!probes.is_empty());

    for provider in PROVIDERS {
        for probe in probes {
            let path = probe["path"].as_str().expect("probe path");
            let expect = probe["expect"].as_str().expect("probe expectation");
            let deployment = deployment(provider, "owner").await;
            let status = deployment
                .oneshot(request("GET", path, None, None))
                .await
                .expect("router response")
                .status();
            assert!(
                !status.is_server_error(),
                "GET {path} answered {status} under '{provider}'; the post-deploy \
                 gate would report this deployment as broken"
            );
            match expect {
                "public" => assert_eq!(status, StatusCode::OK, "GET {path} ({provider})"),
                "authenticated" => assert!(
                    status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
                    "GET {path} answered {status} without credentials under \
                     '{provider}'; the post-deploy gate expects 401 or 403"
                ),
                other => panic!("{path} declares an unknown expectation '{other}'"),
            }
        }
    }
}
