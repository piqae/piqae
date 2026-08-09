//! Fail-closed negotiation for optional document rendering on a node.
//!
//! This module deliberately does not define a wire message or a printer RAW
//! mode. The control plane must always retain an ordinary PDF fallback until
//! the node proves that the exact deterministic renderer contract is present.

use piqae_document_renderer::{
    DocumentSpecV1, RENDERER_VERSION, RenderLimits, SPEC_VERSION, render,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// Changes whenever otherwise-valid input could produce different PDF bytes.
pub const RENDERER_ABI: &str = "piqae.compact-pdf/v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDocumentCapabilities {
    pub negotiation_version: u16,
    pub renderer_abi: String,
    pub renderer_build: String,
    pub spec_versions: Vec<String>,
    pub deterministic: bool,
    pub max_input_bytes: u64,
    pub max_output_bytes: u64,
    pub max_pages: u32,
}

impl NodeDocumentCapabilities {
    #[must_use]
    pub fn local() -> Self {
        let limits = RenderLimits::default();
        Self {
            negotiation_version: 1,
            renderer_abi: RENDERER_ABI.into(),
            renderer_build: RENDERER_VERSION.into(),
            spec_versions: vec![SPEC_VERSION.into()],
            deterministic: true,
            max_input_bytes: 4 * 1024 * 1024,
            max_output_bytes: u64::try_from(limits.max_output_bytes).unwrap_or(u64::MAX),
            max_pages: u32::try_from(limits.max_pages).unwrap_or(u32::MAX),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NodeRenderRequirement {
    pub negotiation_version: u16,
    pub renderer_abi: String,
    pub renderer_build: String,
    pub spec_version: String,
    pub input_bytes: u64,
    pub maximum_pdf_bytes: u64,
    pub maximum_pages: u32,
    /// Digest from a trusted render of the same immutable spec and input.
    pub expected_pdf_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FallbackReason {
    NegotiationVersion,
    NonDeterministicRenderer,
    RendererAbi,
    RendererBuild,
    SpecVersion,
    InputLimit,
    OutputLimit,
    PageLimit,
    InvalidExpectedDigest,
    RenderFailed,
    DigestMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderPlan {
    NodeWithServerPdfFallback,
    ServerPdf { reason: FallbackReason },
}

/// Selects node rendering only on an exact, bounded capability match.
#[must_use]
pub fn negotiate(
    capabilities: &NodeDocumentCapabilities,
    requirement: &NodeRenderRequirement,
) -> RenderPlan {
    let fallback = |reason| RenderPlan::ServerPdf { reason };
    if capabilities.negotiation_version != 1 || requirement.negotiation_version != 1 {
        return fallback(FallbackReason::NegotiationVersion);
    }
    if !capabilities.deterministic {
        return fallback(FallbackReason::NonDeterministicRenderer);
    }
    if capabilities.renderer_abi != requirement.renderer_abi {
        return fallback(FallbackReason::RendererAbi);
    }
    if capabilities.renderer_build != requirement.renderer_build {
        return fallback(FallbackReason::RendererBuild);
    }
    if !capabilities
        .spec_versions
        .iter()
        .any(|version| version == &requirement.spec_version)
    {
        return fallback(FallbackReason::SpecVersion);
    }
    if requirement.input_bytes > capabilities.max_input_bytes {
        return fallback(FallbackReason::InputLimit);
    }
    if requirement.maximum_pdf_bytes > capabilities.max_output_bytes {
        return fallback(FallbackReason::OutputLimit);
    }
    if requirement.maximum_pages > capabilities.max_pages {
        return fallback(FallbackReason::PageLimit);
    }
    if !is_sha256(&requirement.expected_pdf_sha256) {
        return fallback(FallbackReason::InvalidExpectedDigest);
    }
    RenderPlan::NodeWithServerPdfFallback
}

#[derive(Debug, Eq, PartialEq)]
pub enum NodeRenderResult {
    Pdf(Vec<u8>),
    UseServerPdf { reason: FallbackReason },
}

/// Renders only after negotiation and verifies byte-for-byte deterministic
/// parity. Any failure visibly selects the retained server PDF.
#[must_use]
pub fn render_or_fallback(
    capabilities: &NodeDocumentCapabilities,
    requirement: &NodeRenderRequirement,
    spec: &DocumentSpecV1,
    input: &Value,
) -> NodeRenderResult {
    if let RenderPlan::ServerPdf { reason } = negotiate(capabilities, requirement) {
        return NodeRenderResult::UseServerPdf { reason };
    }
    let actual_input_bytes = serde_json::to_vec(input)
        .ok()
        .and_then(|encoded| u64::try_from(encoded.len()).ok());
    if actual_input_bytes
        .is_none_or(|bytes| bytes > requirement.input_bytes || bytes > capabilities.max_input_bytes)
    {
        return NodeRenderResult::UseServerPdf {
            reason: FallbackReason::InputLimit,
        };
    }
    let limits = RenderLimits {
        max_pages: usize::try_from(requirement.maximum_pages).unwrap_or(usize::MAX),
        max_output_bytes: usize::try_from(requirement.maximum_pdf_bytes).unwrap_or(usize::MAX),
        ..RenderLimits::default()
    };
    let Ok(pdf) = render(spec, input, limits) else {
        return NodeRenderResult::UseServerPdf {
            reason: FallbackReason::RenderFailed,
        };
    };
    let actual = format!("{:x}", Sha256::digest(&pdf));
    if !actual.eq_ignore_ascii_case(&requirement.expected_pdf_sha256) {
        return NodeRenderResult::UseServerPdf {
            reason: FallbackReason::DigestMismatch,
        };
    }
    NodeRenderResult::Pdf(pdf)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "deterministic fixture construction should fail at its source"
)]
mod tests {
    use super::*;
    use piqae_document_renderer::{Node, Page, PageSize, TextValue};

    fn spec() -> DocumentSpecV1 {
        DocumentSpecV1 {
            spec_version: SPEC_VERSION.into(),
            page: Page {
                size: PageSize::A5,
                margin_mm: 10.0,
            },
            body: vec![Node::Text {
                value: TextValue::Binding {
                    pointer: "/order".into(),
                },
                font_size: 10.0,
            }],
        }
    }

    fn requirement(pdf: &[u8]) -> NodeRenderRequirement {
        NodeRenderRequirement {
            negotiation_version: 1,
            renderer_abi: RENDERER_ABI.into(),
            renderer_build: RENDERER_VERSION.into(),
            spec_version: SPEC_VERSION.into(),
            input_bytes: 32,
            maximum_pdf_bytes: 1024 * 1024,
            maximum_pages: 2,
            expected_pdf_sha256: format!("{:x}", Sha256::digest(pdf)),
        }
    }

    #[test]
    fn exact_contract_renders_identical_pdf() {
        let input = serde_json::json!({"order":"#1001"});
        let expected = render(&spec(), &input, RenderLimits::default()).expect("fixture render");
        assert_eq!(
            render_or_fallback(
                &NodeDocumentCapabilities::local(),
                &requirement(&expected),
                &spec(),
                &input
            ),
            NodeRenderResult::Pdf(expected)
        );
    }

    #[test]
    fn every_capability_mismatch_uses_server_pdf() {
        let input = serde_json::json!({"order":"#1001"});
        let expected = render(&spec(), &input, RenderLimits::default()).expect("fixture render");
        let requirement = requirement(&expected);
        let mut cases = Vec::new();
        let mut capabilities = NodeDocumentCapabilities::local();
        capabilities.renderer_build = "next".into();
        cases.push(capabilities);
        let mut capabilities = NodeDocumentCapabilities::local();
        capabilities.deterministic = false;
        cases.push(capabilities);
        let mut capabilities = NodeDocumentCapabilities::local();
        capabilities.max_input_bytes = 1;
        cases.push(capabilities);
        for capabilities in cases {
            assert!(matches!(
                render_or_fallback(&capabilities, &requirement, &spec(), &input),
                NodeRenderResult::UseServerPdf { .. }
            ));
        }
    }

    #[test]
    fn digest_disagreement_never_reaches_printing() {
        let input = serde_json::json!({"order":"#1001"});
        let mut requirement = requirement(b"different trusted PDF");
        requirement.expected_pdf_sha256 = format!("{:x}", Sha256::digest(b"different"));
        assert_eq!(
            render_or_fallback(
                &NodeDocumentCapabilities::local(),
                &requirement,
                &spec(),
                &input
            ),
            NodeRenderResult::UseServerPdf {
                reason: FallbackReason::DigestMismatch
            }
        );
    }

    #[test]
    fn valid_uppercase_sha256_is_normalized_for_comparison() {
        let input = serde_json::json!({"order":"#1001"});
        let expected = render(&spec(), &input, RenderLimits::default()).expect("fixture render");
        let mut requirement = requirement(&expected);
        requirement.expected_pdf_sha256 = requirement.expected_pdf_sha256.to_ascii_uppercase();
        assert_eq!(
            render_or_fallback(
                &NodeDocumentCapabilities::local(),
                &requirement,
                &spec(),
                &input
            ),
            NodeRenderResult::Pdf(expected)
        );
    }
}
