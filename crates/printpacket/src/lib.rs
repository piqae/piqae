//! Vendor-neutral contracts for small, data-driven business print documents.
//!
//! `PrintPacket` describes document semantics. It deliberately does not define
//! discovery, queues, leases, printer drivers, or proof of physical delivery.
//! A host can render locally without a network connection, render in a service,
//! or negotiate a compatible renderer on a remote node.

use std::collections::{BTreeMap, BTreeSet};

pub use printpacket_renderer::{
    BarcodeSymbology, Color, DateFormat, Edges, Expr, ImageFit, Inline, Media, Node, Orientation,
    PageSize, PrintPacketV1 as DocumentV1, QrCorrection, Region, RenderError, RenderLimits,
    RenderOutput, ResolvedResources, Resource, StringOperation, TableColumn, TableStyle, TextAlign,
    TextStyle, Theme, render, render_with_metrics, render_with_resources, validate,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use thiserror::Error;

/// Canonical, implementation-independent document identifier.
pub const DOCUMENT_V1: &str = "printpacket/v1";
/// Stable deterministic PDF output contract for the initial conformance set.
pub const PDF_BASE14_V1: &str = "printpacket.pdf-base14/v1";
/// Identifies the exact public fixtures that compatible renderers must pass.
pub const CONFORMANCE_CORE_V1: &str = "printpacket.conformance/core-v1";
/// Corrected inline whitespace and Base-14 glyph-metric conformance fixtures.
pub const CONFORMANCE_CORE_V2: &str = "printpacket.conformance/core-v2";
/// Canonical JSON algorithm used for template, data, and cache identities.
pub const CANONICAL_JSON_V1: &str = "printpacket.canonical-json/v1";
/// Typed, cross-runtime data encoding used by render cache identities.
pub const CANONICAL_DATA_V1: &str = "printpacket.canonical-data/v1";
const MAX_CANONICAL_DATA_BYTES: usize = 4 * 1024 * 1024;
const MAX_CANONICAL_DATA_DEPTH: usize = 128;
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PacketError {
    #[error("unsupported PrintPacket document version: {0}")]
    UnsupportedVersion(String),
    #[error("invalid PrintPacket document: {0}")]
    InvalidDocument(String),
    #[error("invalid PrintPacket data: {0}")]
    InvalidData(&'static str),
}

/// Independently negotiable semantic features. New feature identifiers are
/// additive; a renderer must reject a required feature it does not advertise.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    MediaPaged,
    MediaContinuous,
    MediaLabel,
    LayoutFlow,
    LayoutGrid,
    LayoutTable,
    LayoutRegions,
    LayoutKeepTogether,
    DataExpressions,
    DataRepeat,
    ImageJpeg,
    BarcodeQr,
    BarcodeCode128,
    TypographyBase14Windows1252,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputKind {
    Pdf,
    PrinterNative,
}

/// A renderer target is explicit. Printer-native output is always bound to a
/// reviewed language/profile and must never be selected from MIME guessing.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutputTarget {
    Pdf {
        profile: String,
    },
    PrinterNative {
        language: String,
        profile: String,
        dpi: u16,
        printable_width_dots: u32,
    },
}

impl OutputTarget {
    #[must_use]
    pub fn pdf_v1() -> Self {
        Self::Pdf {
            profile: PDF_BASE14_V1.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PacketLimits {
    pub max_template_bytes: u64,
    pub max_data_bytes: u64,
    pub max_output_bytes: u64,
    pub max_pages: u32,
    pub max_resources: u32,
    pub max_resource_bytes: u64,
    pub max_total_resource_bytes: u64,
}

impl Default for PacketLimits {
    fn default() -> Self {
        let renderer = RenderLimits::default();
        Self {
            max_template_bytes: 1024 * 1024,
            max_data_bytes: 4 * 1024 * 1024,
            max_output_bytes: u64::try_from(renderer.max_output_bytes).unwrap_or(u64::MAX),
            max_pages: u32::try_from(renderer.max_pages).unwrap_or(u32::MAX),
            max_resources: 100,
            max_resource_bytes: 4 * 1024 * 1024,
            // Fits a complete base64-encoded packet plus maximum template/data
            // inside the reference native ABI's 24 MiB request bound.
            max_total_resource_bytes: 12 * 1024 * 1024,
        }
    }
}

/// Portable, cache-safe analysis of one immutable template.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateManifest {
    pub standard: String,
    pub specification_version: String,
    pub canonical_json: String,
    pub canonical_sha256: String,
    pub canonical_bytes: u64,
    pub required_features: BTreeSet<Feature>,
    pub resource_count: u32,
    pub resource_bytes: u64,
}

/// A node or in-process SDK reports semantic support, not just its application
/// version. `implementation_version` remains diagnostic and is never sufficient
/// evidence of conformance by itself.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RendererCapabilities {
    pub negotiation_version: u16,
    pub specification_versions: BTreeSet<String>,
    pub conformance_suites: BTreeSet<String>,
    pub features: BTreeSet<Feature>,
    pub output_targets: Vec<OutputTarget>,
    pub resource_media_types: BTreeSet<String>,
    pub limits: PacketLimits,
    pub persistent_resource_cache: bool,
    pub direct_offline_rendering: bool,
    pub implementation_version: String,
}

impl RendererCapabilities {
    #[must_use]
    pub fn reference_pdf() -> Self {
        Self {
            negotiation_version: 1,
            specification_versions: BTreeSet::from([DOCUMENT_V1.into()]),
            conformance_suites: BTreeSet::from([CONFORMANCE_CORE_V2.into()]),
            features: all_v1_features(),
            output_targets: vec![OutputTarget::pdf_v1()],
            resource_media_types: BTreeSet::from(["image/jpeg".into()]),
            limits: PacketLimits::default(),
            persistent_resource_cache: false,
            direct_offline_rendering: true,
            implementation_version: env!("CARGO_PKG_VERSION").into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderRequirement {
    pub specification_version: String,
    pub conformance_suite: String,
    pub required_features: BTreeSet<Feature>,
    pub output_target: OutputTarget,
    pub template_bytes: u64,
    pub data_bytes: u64,
    pub maximum_output_bytes: u64,
    pub maximum_pages: u32,
    pub resource_count: u32,
    pub maximum_resource_bytes: u64,
    pub resource_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus {
    Compatible,
    NodeUpdateRequired,
    UnsupportedTarget,
    LimitExceeded,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompatibilityReport {
    pub status: CompatibilityStatus,
    pub reason: String,
    pub missing_features: BTreeSet<Feature>,
    pub supported_specification_versions: BTreeSet<String>,
    pub implementation_version: String,
}

/// Require the canonical vendor-neutral v1 identifier.
///
/// # Errors
/// Returns [`PacketError::UnsupportedVersion`] for any other identifier.
pub fn normalize_document(document: DocumentV1) -> Result<DocumentV1, PacketError> {
    match document.format.as_str() {
        DOCUMENT_V1 => Ok(document),
        unsupported => Err(PacketError::UnsupportedVersion(unsupported.into())),
    }
}

/// Parse, normalize, validate, and derive a portable feature/cache manifest.
///
/// # Errors
/// Returns a bounded parse, format, validation, or size error.
pub fn analyze_document(
    specification: &Value,
    limits: RenderLimits,
) -> Result<(DocumentV1, TemplateManifest), PacketError> {
    let document: DocumentV1 = serde_json::from_value(specification.clone())
        .map_err(|error| PacketError::InvalidDocument(error.to_string()))?;
    let document = normalize_document(document)?;
    validate(&document, limits).map_err(|error| PacketError::InvalidDocument(error.to_string()))?;
    let canonical = canonical_document_bytes(&document)?;
    let resource_bytes = document
        .resources
        .values()
        .try_fold(0_u64, |total, resource| {
            let Resource::Image { byte_length, .. } = resource;
            total
                .checked_add(*byte_length)
                .ok_or(PacketError::InvalidData("resource byte total overflow"))
        })?;
    let manifest = TemplateManifest {
        standard: "PrintPacket".into(),
        specification_version: DOCUMENT_V1.into(),
        canonical_json: CANONICAL_JSON_V1.into(),
        canonical_sha256: sha256_hex(&canonical),
        canonical_bytes: u64::try_from(canonical.len())
            .map_err(|_| PacketError::InvalidData("template is too large"))?,
        required_features: required_features(&document),
        resource_count: u32::try_from(document.resources.len())
            .map_err(|_| PacketError::InvalidData("too many resources"))?,
        resource_bytes,
    };
    Ok((document, manifest))
}

/// Create canonical v1 template JSON.
///
/// It uses normalized structs, declaration-order object fields, lexically
/// sorted map keys, UTF-8 JSON strings, and `serde_json` number spelling. The
/// algorithm is versioned and is not claimed to implement RFC 8785.
///
/// # Errors
/// Returns a format or serialization error.
pub fn canonical_document_bytes(document: &DocumentV1) -> Result<Vec<u8>, PacketError> {
    let normalized = normalize_document(document.clone())?;
    serde_json::to_vec(&normalized).map_err(|error| PacketError::InvalidDocument(error.to_string()))
}

/// Canonicalize arbitrary template data with a typed cross-runtime byte format.
///
/// Values use one-byte type tags, decimal collection/string byte lengths, UTF-8
/// strings, lexically sorted object keys, and exactly 16 lowercase hexadecimal
/// IEEE-754 binary64 bits for numbers. Negative zero is normalized to positive
/// zero. Integral values outside JavaScript's exact safe-integer range fail
/// closed so a Rust producer and browser/JavaScript producer cannot assign
/// different values to one cache identity.
///
/// # Errors
/// Returns an error if the supplied value cannot be encoded as bounded JSON.
pub fn canonical_data_bytes(data: &Value) -> Result<Vec<u8>, PacketError> {
    fn append(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), PacketError> {
        if output
            .len()
            .checked_add(bytes.len())
            .is_none_or(|length| length > MAX_CANONICAL_DATA_BYTES)
        {
            return Err(PacketError::InvalidData("canonical data is too large"));
        }
        output.extend_from_slice(bytes);
        Ok(())
    }

    fn length(output: &mut Vec<u8>, value: usize) -> Result<(), PacketError> {
        append(output, value.to_string().as_bytes())?;
        append(output, b":")
    }

    fn encode(value: &Value, output: &mut Vec<u8>, depth: usize) -> Result<(), PacketError> {
        if depth > MAX_CANONICAL_DATA_DEPTH {
            return Err(PacketError::InvalidData("canonical data nesting"));
        }
        match value {
            Value::Null => append(output, b"n"),
            Value::Bool(false) => append(output, b"f"),
            Value::Bool(true) => append(output, b"t"),
            Value::String(value) => {
                append(output, b"s")?;
                length(output, value.len())?;
                append(output, value.as_bytes())
            }
            Value::Number(value) => {
                let mut number = value
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .ok_or(PacketError::InvalidData("data number"))?;
                if number.fract() == 0.0 && number.abs() > MAX_SAFE_INTEGER {
                    return Err(PacketError::InvalidData(
                        "integer is outside the cross-runtime safe range",
                    ));
                }
                if number == 0.0 {
                    number = 0.0;
                }
                append(output, format!("d{:016x}", number.to_bits()).as_bytes())
            }
            Value::Array(values) => {
                append(output, b"a")?;
                length(output, values.len())?;
                for value in values {
                    encode(value, output, depth + 1)?;
                }
                Ok(())
            }
            Value::Object(values) => {
                append(output, b"o")?;
                length(output, values.len())?;
                let sorted = values.iter().collect::<BTreeMap<_, _>>();
                for (key, value) in sorted {
                    encode(&Value::String(key.clone()), output, depth + 1)?;
                    encode(value, output, depth + 1)?;
                }
                Ok(())
            }
        }
    }

    let mut output = CANONICAL_DATA_V1.as_bytes().to_vec();
    append(&mut output, b"\0")?;
    encode(data, &mut output, 0)?;
    Ok(output)
}

/// Domain-separated render cache identity. Hosts may cache the output only
/// while this exact key, renderer conformance suite, and resource digests match.
///
/// # Errors
/// Returns an error if the data or output target cannot be canonicalized.
pub fn render_cache_key(
    manifest: &TemplateManifest,
    data: &Value,
    output_target: &OutputTarget,
) -> Result<String, PacketError> {
    let data = canonical_data_bytes(data)?;
    let target =
        serde_json::to_vec(output_target).map_err(|_| PacketError::InvalidData("output target"))?;
    let mut hash = Sha256::new();
    hash.update(b"printpacket.render-cache/v1\0");
    hash.update(manifest.canonical_sha256.as_bytes());
    hash.update(b"\0");
    hash.update(CONFORMANCE_CORE_V2.as_bytes());
    hash.update(b"\0");
    hash.update(target);
    hash.update(b"\0");
    hash.update(data);
    Ok(hex_lower(hash.finalize().as_slice()))
}

#[must_use]
pub fn negotiate(
    capabilities: &RendererCapabilities,
    requirement: &RenderRequirement,
) -> CompatibilityReport {
    let missing_features = requirement
        .required_features
        .difference(&capabilities.features)
        .cloned()
        .collect::<BTreeSet<_>>();
    let common = |status, reason: &str| CompatibilityReport {
        status,
        reason: reason.into(),
        missing_features: missing_features.clone(),
        supported_specification_versions: capabilities.specification_versions.clone(),
        implementation_version: capabilities.implementation_version.clone(),
    };
    if capabilities.negotiation_version != 1
        || !capabilities
            .specification_versions
            .contains(&requirement.specification_version)
        || !capabilities
            .conformance_suites
            .contains(&requirement.conformance_suite)
        || !missing_features.is_empty()
    {
        return common(
            CompatibilityStatus::NodeUpdateRequired,
            "renderer_feature_or_version_update_required",
        );
    }
    if !capabilities
        .output_targets
        .iter()
        .any(|target| target == &requirement.output_target)
    {
        return common(
            CompatibilityStatus::UnsupportedTarget,
            "explicit_output_target_unsupported",
        );
    }
    let limits = &capabilities.limits;
    if requirement.template_bytes > limits.max_template_bytes
        || requirement.data_bytes > limits.max_data_bytes
        || requirement.maximum_output_bytes > limits.max_output_bytes
        || requirement.maximum_pages > limits.max_pages
        || requirement.resource_count > limits.max_resources
        || requirement.maximum_resource_bytes > limits.max_resource_bytes
        || requirement.resource_bytes > limits.max_total_resource_bytes
    {
        return common(
            CompatibilityStatus::LimitExceeded,
            "renderer_limit_exceeded",
        );
    }
    common(CompatibilityStatus::Compatible, "compatible")
}

#[must_use]
pub fn required_features(document: &DocumentV1) -> BTreeSet<Feature> {
    let mut features = BTreeSet::from([
        Feature::LayoutFlow,
        Feature::DataExpressions,
        Feature::TypographyBase14Windows1252,
    ]);
    features.insert(match document.media {
        Media::Paged { .. } => Feature::MediaPaged,
        Media::Continuous { .. } => Feature::MediaContinuous,
        Media::Label { .. } => Feature::MediaLabel,
    });
    if document.header.is_some() || document.footer.is_some() {
        features.insert(Feature::LayoutRegions);
    }
    for resource in document.resources.values() {
        if matches!(resource, Resource::Image { media_type, .. } if media_type == "image/jpeg") {
            features.insert(Feature::ImageJpeg);
        }
    }
    collect_nodes(&document.body, &mut features);
    if let Some(region) = &document.header {
        collect_nodes(&region.first, &mut features);
        collect_nodes(&region.default, &mut features);
        collect_nodes(&region.last, &mut features);
    }
    if let Some(region) = &document.footer {
        collect_nodes(&region.first, &mut features);
        collect_nodes(&region.default, &mut features);
        collect_nodes(&region.last, &mut features);
    }
    features
}

fn collect_nodes(nodes: &[Node], features: &mut BTreeSet<Feature>) {
    for node in nodes {
        match node {
            Node::Section { children, .. }
            | Node::Box { children, .. }
            | Node::Stack { children, .. }
            | Node::Row { children, .. } => collect_nodes(children, features),
            Node::Grid { children, .. } => {
                features.insert(Feature::LayoutGrid);
                collect_nodes(children, features);
            }
            Node::Table { empty, .. } => {
                features.insert(Feature::LayoutTable);
                features.insert(Feature::DataRepeat);
                collect_nodes(empty, features);
            }
            Node::Repeat { children, .. } => {
                features.insert(Feature::DataRepeat);
                collect_nodes(children, features);
            }
            Node::DataList {
                header,
                item,
                empty,
                ..
            } => {
                features.insert(Feature::DataRepeat);
                collect_nodes(header, features);
                collect_nodes(item, features);
                collect_nodes(empty, features);
            }
            Node::Conditional {
                then, otherwise, ..
            } => {
                collect_nodes(then, features);
                collect_nodes(otherwise, features);
            }
            Node::KeepTogether { children } => {
                features.insert(Feature::LayoutKeepTogether);
                collect_nodes(children, features);
            }
            Node::Image { .. } | Node::ImageValue { .. } => {
                features.insert(Feature::ImageJpeg);
            }
            Node::Qr { .. } => {
                features.insert(Feature::BarcodeQr);
            }
            Node::Barcode {
                symbology: BarcodeSymbology::Code128,
                ..
            } => {
                features.insert(Feature::BarcodeCode128);
            }
            Node::Paragraph { .. }
            | Node::Heading { .. }
            | Node::Spacer { .. }
            | Node::Divider { .. }
            | Node::PageBreak => {}
        }
    }
}

fn all_v1_features() -> BTreeSet<Feature> {
    BTreeSet::from([
        Feature::MediaPaged,
        Feature::MediaContinuous,
        Feature::MediaLabel,
        Feature::LayoutFlow,
        Feature::LayoutGrid,
        Feature::LayoutTable,
        Feature::LayoutRegions,
        Feature::LayoutKeepTogether,
        Feature::DataExpressions,
        Feature::DataRepeat,
        Feature::ImageJpeg,
        Feature::BarcodeQr,
        Feature::BarcodeCode128,
        Feature::TypographyBase14Windows1252,
    ])
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(Sha256::digest(bytes).as_slice())
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::Sha256;

    fn receipt(format: &str) -> Value {
        json!({
            "format": format,
            "media": {"kind":"continuous","width_mm":80.0,"margins":{"top_mm":2.0,"right_mm":2.0,"bottom_mm":2.0,"left_mm":2.0}},
            "body": [
                {"type":"heading","level":1,"content":[{"type":"text","value":"Receipt"}]},
                {"type":"repeat","items":{"type":"path","path":["lines"]},"children":[
                    {"type":"paragraph","content":[{"type":"value","value":{"type":"current_path","path":["name"]}}]}
                ]},
                {"type":"qr","value":{"type":"path","path":["receipt_url"]},"size_mm":20.0}
            ]
        })
    }

    #[test]
    fn canonical_document_is_analyzed_and_old_identifiers_are_rejected() {
        let (_, neutral) = analyze_document(&receipt(DOCUMENT_V1), RenderLimits::default())
            .unwrap_or_else(|error| panic!("neutral fixture: {error}"));
        assert!(matches!(
            analyze_document(
                &receipt("piqae.business-document/v1"),
                RenderLimits::default()
            ),
            Err(PacketError::UnsupportedVersion(version))
                if version == "piqae.business-document/v1"
        ));
        assert!(
            neutral
                .required_features
                .contains(&Feature::MediaContinuous)
        );
        assert!(neutral.required_features.contains(&Feature::DataRepeat));
        assert!(neutral.required_features.contains(&Feature::BarcodeQr));
    }

    #[test]
    fn canonical_data_is_independent_of_object_insertion_order() {
        let left = json!({"b":2,"a":{"d":4,"c":3}});
        let right = json!({"a":{"c":3,"d":4},"b":2});
        assert_eq!(
            canonical_data_bytes(&left).unwrap_or_default(),
            canonical_data_bytes(&right).unwrap_or_default()
        );
    }

    #[test]
    fn canonical_data_has_exact_cross_runtime_numeric_bytes() {
        let encoded = canonical_data_bytes(&json!({"n": 1, "z": -0.0, "text": "café"}))
            .unwrap_or_else(|error| panic!("canonical data: {error}"));
        assert_eq!(
            encoded,
            b"printpacket.canonical-data/v1\0o3:s1:nd3ff0000000000000s4:texts5:caf\xc3\xa9s1:zd0000000000000000"
        );
        assert_eq!(
            canonical_data_bytes(&json!(1)).unwrap_or_default(),
            canonical_data_bytes(&json!(1.0)).unwrap_or_default()
        );
        assert_eq!(
            canonical_data_bytes(&json!(9_007_199_254_740_992_u64)),
            Err(PacketError::InvalidData(
                "integer is outside the cross-runtime safe range"
            ))
        );
    }

    #[test]
    fn negotiation_reports_actionable_update_and_target_failures() {
        let capabilities = RendererCapabilities::reference_pdf();
        let mut requirement = RenderRequirement {
            specification_version: DOCUMENT_V1.into(),
            conformance_suite: CONFORMANCE_CORE_V2.into(),
            required_features: BTreeSet::from([Feature::MediaLabel, Feature::BarcodeCode128]),
            output_target: OutputTarget::pdf_v1(),
            template_bytes: 1024,
            data_bytes: 512,
            maximum_output_bytes: 4096,
            maximum_pages: 1,
            resource_count: 0,
            maximum_resource_bytes: 0,
            resource_bytes: 0,
        };
        assert_eq!(
            negotiate(&capabilities, &requirement).status,
            CompatibilityStatus::Compatible
        );
        requirement.required_features.insert(Feature::ImageJpeg);
        let mut old = capabilities.clone();
        old.features.remove(&Feature::ImageJpeg);
        let report = negotiate(&old, &requirement);
        assert_eq!(report.status, CompatibilityStatus::NodeUpdateRequired);
        assert_eq!(
            report.missing_features,
            BTreeSet::from([Feature::ImageJpeg])
        );

        requirement.required_features.remove(&Feature::ImageJpeg);
        requirement.output_target = OutputTarget::PrinterNative {
            language: "zpl".into(),
            profile: "zpl-raster/v1".into(),
            dpi: 203,
            printable_width_dots: 812,
        };
        assert_eq!(
            negotiate(&capabilities, &requirement).status,
            CompatibilityStatus::UnsupportedTarget
        );
    }

    #[test]
    fn public_conformance_fixtures_match_exact_golden_outputs() {
        let expected: Value = serde_json::from_str(include_str!(
            "../../../standards/printpacket/conformance/expected.json"
        ))
        .unwrap_or_else(|error| panic!("expected manifest: {error}"));
        for (name, source) in [
            (
                "receipt-80mm",
                include_str!("../../../standards/printpacket/conformance/receipt-80mm.json"),
            ),
            (
                "production-label-100x50",
                include_str!(
                    "../../../standards/printpacket/conformance/production-label-100x50.json"
                ),
            ),
            (
                "invoice-a4",
                include_str!("../../../standards/printpacket/conformance/invoice-a4.json"),
            ),
        ] {
            let fixture: Value =
                serde_json::from_str(source).unwrap_or_else(|error| panic!("{name} JSON: {error}"));
            let (document, manifest) =
                analyze_document(&fixture["template"], RenderLimits::default())
                    .unwrap_or_else(|error| panic!("{name} template: {error}"));
            let output = render_with_metrics(
                &document,
                &fixture["data"],
                &ResolvedResources::default(),
                RenderLimits::default(),
            )
            .unwrap_or_else(|error| panic!("{name} render: {error}"));
            let cache_key = render_cache_key(&manifest, &fixture["data"], &OutputTarget::pdf_v1())
                .unwrap_or_else(|error| panic!("{name} cache: {error}"));
            let golden = &expected["fixtures"][name];
            assert_eq!(golden["template_sha256"], manifest.canonical_sha256);
            assert_eq!(golden["cache_key"], cache_key);
            assert_eq!(
                golden["output_sha256"],
                format!("{:x}", Sha256::digest(&output.pdf))
            );
            assert_eq!(golden["output_bytes"], output.pdf.len());
            assert_eq!(golden["pages"], output.page_count);
        }
    }
}
