//! Fail-closed negotiation for optional document rendering on a node.
//!
//! This module deliberately does not define a wire message or a printer RAW
//! mode. The control plane must always retain an ordinary PDF fallback until
//! the node proves that the exact deterministic renderer contract is present.

use piqae_document_renderer::{
    BUSINESS_DOCUMENT_FORMAT, BusinessDocumentV1, PRINT_PACKET_DOCUMENT_FORMAT, RENDERER_VERSION,
    RenderLimits, ResolvedResources, render, render_with_resources,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// Changes whenever otherwise-valid input could produce different PDF bytes.
pub const RENDERER_ABI: &str = "piqae.business-document-pdf/v1";

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
    pub resource_abi: String,
    pub persistent_resource_cache: bool,
    pub supported_image_media_types: Vec<String>,
    /// Remains false until the deterministic renderer has a reviewed embedded
    /// font ABI. Cached font bytes must never be mistaken for render support.
    pub font_rendering: bool,
    pub max_resource_bytes: u64,
    pub max_resources: u32,
}

impl NodeDocumentCapabilities {
    #[must_use]
    pub fn local() -> Self {
        let limits = RenderLimits::default();
        Self {
            negotiation_version: 1,
            renderer_abi: RENDERER_ABI.into(),
            renderer_build: RENDERER_VERSION.into(),
            spec_versions: vec![
                PRINT_PACKET_DOCUMENT_FORMAT.into(),
                BUSINESS_DOCUMENT_FORMAT.into(),
            ],
            deterministic: true,
            max_input_bytes: 4 * 1024 * 1024,
            max_output_bytes: u64::try_from(limits.max_output_bytes).unwrap_or(u64::MAX),
            max_pages: u32::try_from(limits.max_pages).unwrap_or(u32::MAX),
            resource_abi: crate::document_resources::RESOURCE_ABI.into(),
            persistent_resource_cache: false,
            supported_image_media_types: vec!["image/jpeg".into()],
            font_rendering: false,
            max_resource_bytes: 4 * 1024 * 1024,
            max_resources: 100,
        }
    }

    #[must_use]
    pub const fn with_persistent_resource_cache(mut self, maximum_resource_bytes: u64) -> Self {
        self.persistent_resource_cache = true;
        self.max_resource_bytes = maximum_resource_bytes;
        self
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
    ResourceUnavailable,
}

/// Resource-aware equivalent of [`render_or_fallback`].
///
/// Only the renderer's explicit JPEG ABI is passed through. Fonts and all
/// other cached byte types remain non-renderable and select the server PDF.
#[must_use]
pub fn render_with_resources_or_fallback(
    capabilities: &NodeDocumentCapabilities,
    requirement: &NodeRenderRequirement,
    spec: &BusinessDocumentV1,
    input: &Value,
    resources: &ResolvedResources,
) -> NodeRenderResult {
    if !spec.resources.is_empty() && !capabilities.persistent_resource_cache {
        return NodeRenderResult::UseServerPdf {
            reason: FallbackReason::ResourceUnavailable,
        };
    }
    if u32::try_from(spec.resources.len()).unwrap_or(u32::MAX) > capabilities.max_resources {
        return NodeRenderResult::UseServerPdf {
            reason: FallbackReason::ResourceUnavailable,
        };
    }
    for resource in spec.resources.values() {
        let piqae_document_renderer::Resource::Image {
            media_type,
            byte_length,
            ..
        } = resource;
        if !capabilities
            .supported_image_media_types
            .iter()
            .any(|supported| supported == media_type)
            || *byte_length > capabilities.max_resource_bytes
        {
            return NodeRenderResult::UseServerPdf {
                reason: FallbackReason::ResourceUnavailable,
            };
        }
    }
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
    let Ok(pdf) = render_with_resources(spec, input, resources, limits) else {
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
    if requirement.maximum_pages == 0 || requirement.maximum_pages > capabilities.max_pages {
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
    spec: &BusinessDocumentV1,
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
    use piqae_document_renderer::{
        Edges, Expr, Inline, Media, Node, Orientation, PageSize, TextStyle, Theme,
    };
    use std::collections::BTreeMap;

    fn spec() -> BusinessDocumentV1 {
        BusinessDocumentV1 {
            format: BUSINESS_DOCUMENT_FORMAT.into(),
            media: Media::Paged {
                size: PageSize::A5,
                orientation: Orientation::Portrait,
                margins: Edges {
                    top_mm: 10.0,
                    right_mm: 10.0,
                    bottom_mm: 10.0,
                    left_mm: 10.0,
                },
            },
            theme: Theme::default(),
            resources: BTreeMap::new(),
            header: None,
            body: vec![Node::Paragraph {
                content: vec![Inline::Value {
                    value: Expr::Path {
                        path: vec!["order".into()],
                    },
                    style: TextStyle::default(),
                }],
                style: TextStyle::default(),
            }],
            footer: None,
        }
    }

    fn requirement(pdf: &[u8]) -> NodeRenderRequirement {
        NodeRenderRequirement {
            negotiation_version: 1,
            renderer_abi: RENDERER_ABI.into(),
            renderer_build: RENDERER_VERSION.into(),
            spec_version: BUSINESS_DOCUMENT_FORMAT.into(),
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
    fn authoritative_completed_page_count_is_the_node_render_bound() {
        let input = serde_json::json!({"order":"#1001"});
        let expected = render(&spec(), &input, RenderLimits::default()).expect("fixture render");
        let mut requirement = requirement(&expected);
        requirement.maximum_pages = 1;
        assert!(matches!(
            render_or_fallback(
                &NodeDocumentCapabilities::local(),
                &requirement,
                &spec(),
                &input
            ),
            NodeRenderResult::Pdf(_)
        ));

        // This was the installed agent's old guessed requirement. It exceeds
        // the renderer capability and therefore made every offer fall back.
        requirement.maximum_pages = 10_000;
        assert_eq!(
            render_or_fallback(
                &NodeDocumentCapabilities::local(),
                &requirement,
                &spec(),
                &input
            ),
            NodeRenderResult::UseServerPdf {
                reason: FallbackReason::PageLimit
            }
        );
    }

    #[test]
    fn canonical_and_frozen_alias_formats_negotiate_exactly() {
        let input = serde_json::json!({"order":"#1001"});
        let capabilities = NodeDocumentCapabilities::local();
        assert_eq!(
            capabilities.spec_versions,
            [PRINT_PACKET_DOCUMENT_FORMAT, BUSINESS_DOCUMENT_FORMAT]
        );
        for format in [PRINT_PACKET_DOCUMENT_FORMAT, BUSINESS_DOCUMENT_FORMAT] {
            let mut document = spec();
            document.format = format.into();
            let expected = render(&document, &input, RenderLimits::default()).expect("render");
            let mut requirement = requirement(&expected);
            requirement.spec_version = format.into();
            assert_eq!(
                render_or_fallback(&capabilities, &requirement, &document, &input),
                NodeRenderResult::Pdf(expected)
            );
        }
    }

    #[test]
    fn legacy_offer_without_authoritative_pages_falls_back_before_rendering() {
        let input = serde_json::json!({"order":"#1001"});
        let expected = render(&spec(), &input, RenderLimits::default()).expect("render");
        let mut requirement = requirement(&expected);
        requirement.maximum_pages = 0;
        assert_eq!(
            render_or_fallback(
                &NodeDocumentCapabilities::local(),
                &requirement,
                &spec(),
                &input
            ),
            NodeRenderResult::UseServerPdf {
                reason: FallbackReason::PageLimit
            }
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
    fn resource_capability_mismatch_falls_back_before_rendering() {
        let input = serde_json::json!({"order":"#1001"});
        let expected = render(&spec(), &input, RenderLimits::default()).expect("fixture render");
        let mut document = spec();
        document.resources.insert(
            "logo".into(),
            piqae_document_renderer::Resource::Image {
                digest: format!("sha256:{}", "a".repeat(64)),
                media_type: "image/jpeg".into(),
                byte_length: 10,
            },
        );
        let mut capabilities = NodeDocumentCapabilities::local().with_persistent_resource_cache(9);
        capabilities.supported_image_media_types = vec!["image/jpeg".into()];
        assert_eq!(
            render_with_resources_or_fallback(
                &capabilities,
                &requirement(&expected),
                &document,
                &input,
                &ResolvedResources::default(),
            ),
            NodeRenderResult::UseServerPdf {
                reason: FallbackReason::ResourceUnavailable
            }
        );
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
