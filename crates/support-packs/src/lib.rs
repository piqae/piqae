//! Declarative, non-executable printer support packs.
//!
//! Packs normalize evidence-backed driver capabilities. They are never an
//! execution plugin: command lines, native blobs and changes to a profile's
//! safe overrides are intentionally absent from the format.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use piqae_domain::{
    DriverFingerprint, NativePrinterOption, SemanticNativeResolution, SemanticPrinterCapabilities,
    SupportPackProvenance,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PACK_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FILES: usize = 128;
const MAX_RULES: usize = 512;
const MAX_FIXTURE_DEPTH: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u16,
    pub pack_id: String,
    pub pack_version: String,
    pub vendor: String,
    pub family: String,
    pub maintainers: Vec<String>,
    #[serde(default)]
    pub selectors: Vec<Selector>,
    /// Exact selectors for an application-bundled printer adapter. These are
    /// data-only fingerprints; support packs never load or distribute the
    /// adapter binary itself.
    #[serde(default)]
    pub adapter_selectors: Vec<AdapterSelector>,
    pub platforms: Vec<Platform>,
    pub facets: Vec<String>,
    pub evidence: EvidenceTier,
    pub mappings: Vec<String>,
    pub conformance: Vec<String>,
    #[serde(default)]
    pub fixtures: Vec<String>,
    pub license: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Selector {
    /// SHA-256 of the installed driver package, lowercase hexadecimal.
    pub driver_package_sha256: String,
    pub driver_id: String,
    pub driver_version: String,
    #[serde(default)]
    pub device_id: Option<String>,
    #[serde(default)]
    pub firmware_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdapterSelector {
    pub platform: Platform,
    /// Stable reverse-DNS identifier owned by the adapter publisher.
    pub adapter_id: String,
    /// Exact bundled adapter or vendor SDK version.
    pub adapter_version: String,
    #[serde(default)]
    pub device_family: Option<String>,
    #[serde(default)]
    pub firmware_version: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    CupsIpp,
    IosAirPrint,
    IosNetwork,
    IosBluetoothLe,
    IosExternalAccessory,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceTier {
    Discovered,
    Mapped,
    ReplayTested,
    PhysicallyCertified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MappingFile {
    pub schema_version: u16,
    pub rules: Vec<MappingRule>,
}

/// A bounded lookup from display-safe native capability values to normalized
/// semantic values. The node's platform adapter remains responsible for driver
/// validation and job-scoped application.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MappingRule {
    pub platform: Platform,
    pub native_capability_key: String,
    pub semantic_facet: String,
    pub choices: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConformanceFile {
    pub schema_version: u16,
    pub cases: Vec<ConformanceCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConformanceCase {
    pub name: String,
    pub platform: Platform,
    pub native_capability_key: String,
    pub native_choice: String,
    #[serde(default)]
    pub expected_semantic_facet: Option<String>,
    #[serde(default)]
    pub expected_semantic_choice: Option<String>,
    #[serde(default)]
    pub expected_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedPack {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub mappings: Vec<MappingRule>,
    /// Digest of the canonical inventory, which covers every regular pack file
    /// except the detached `SIGNATURE` file.
    pub digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterFingerprint {
    pub platform: Platform,
    pub driver_package_sha256: String,
    pub driver_id: String,
    pub driver_version: String,
    pub device_id: Option<String>,
    pub firmware_version: Option<String>,
}

/// Display-safe evidence supplied by an embedded host for an adapter that is
/// compiled into that application. It intentionally excludes device serials,
/// credentials and executable material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterFingerprint {
    pub platform: Platform,
    pub adapter_id: String,
    pub adapter_version: String,
    pub device_family: Option<String>,
    pub firmware_version: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TrustPolicy {
    pub pinned_digests: BTreeSet<[u8; 32]>,
    pub verifying_keys: Vec<VerifyingKey>,
}

#[derive(Debug, Clone, Default)]
pub struct RegistryConfig {
    pub pack_directories: Vec<PathBuf>,
    pub pinned_digest_hex: Vec<String>,
    pub ed25519_public_key_hex: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SupportPackRegistry {
    packs: Vec<LoadedPack>,
}

#[derive(Debug, Error)]
pub enum PackError {
    #[error("support-pack I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("invalid support pack: {0}")]
    Invalid(String),
    #[error("support pack is not trusted")]
    Untrusted,
    #[error("detached support-pack signature is invalid")]
    InvalidSignature,
    #[error("no trusted support pack matches the printer fingerprint")]
    NoMatch,
    #[error("multiple trusted support packs match the printer fingerprint: {0:?}")]
    Ambiguous(Vec<String>),
}

impl SupportPackRegistry {
    /// Loads all configured packs. A configured but invalid pack prevents the
    /// registry from starting; silently skipping it could alter capabilities
    /// across restarts.
    ///
    /// # Errors
    ///
    /// Returns [`PackError`] for invalid trust material or any invalid pack.
    pub fn load(config: &RegistryConfig) -> Result<Self, PackError> {
        let pinned_digests = config
            .pinned_digest_hex
            .iter()
            .map(|value| parse_array::<32>(value, "pinned digest"))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let verifying_keys = config
            .ed25519_public_key_hex
            .iter()
            .map(|value| {
                VerifyingKey::from_bytes(&parse_array::<32>(value, "Ed25519 public key")?)
                    .map_err(|_| PackError::Invalid("invalid Ed25519 public key".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let trust = TrustPolicy {
            pinned_digests,
            verifying_keys,
        };
        let packs = config
            .pack_directories
            .iter()
            .map(|directory| load_pack(directory, &trust))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { packs })
    }

    /// Projects observed display-safe choices through one exact trusted pack.
    /// An absent package digest or driver version returns an empty projection.
    ///
    /// # Errors
    ///
    /// Returns [`PackError::Ambiguous`] when multiple packs match. No match is
    /// intentionally represented by an empty projection.
    pub fn normalize(
        &self,
        fingerprint: Option<&DriverFingerprint>,
        native_options: &BTreeMap<String, NativePrinterOption>,
    ) -> Result<SemanticPrinterCapabilities, PackError> {
        let Some(fingerprint) = fingerprint.and_then(to_printer_fingerprint) else {
            return Ok(SemanticPrinterCapabilities::default());
        };
        let pack = match select_pack(&self.packs, &fingerprint) {
            Ok(pack) => pack,
            Err(PackError::NoMatch) => return Ok(SemanticPrinterCapabilities::default()),
            Err(error) => return Err(error),
        };
        Ok(project_capabilities(
            pack,
            fingerprint.platform,
            native_options,
        ))
    }

    /// Projects options reported by an exact application-bundled adapter
    /// through one trusted pack. No match is an empty projection and multiple
    /// matches fail closed.
    ///
    /// # Errors
    ///
    /// Returns [`PackError::Ambiguous`] when multiple packs match.
    pub fn normalize_adapter(
        &self,
        fingerprint: &AdapterFingerprint,
        native_options: &BTreeMap<String, NativePrinterOption>,
    ) -> Result<SemanticPrinterCapabilities, PackError> {
        let pack = match select_adapter_pack(&self.packs, fingerprint) {
            Ok(pack) => pack,
            Err(PackError::NoMatch) => return Ok(SemanticPrinterCapabilities::default()),
            Err(error) => return Err(error),
        };
        Ok(project_capabilities(
            pack,
            fingerprint.platform,
            native_options,
        ))
    }
}

fn project_capabilities(
    pack: &LoadedPack,
    platform: Platform,
    native_options: &BTreeMap<String, NativePrinterOption>,
) -> SemanticPrinterCapabilities {
    let mut values = BTreeMap::<String, BTreeSet<String>>::new();
    let mut resolutions =
        BTreeMap::<String, BTreeMap<String, Option<SemanticNativeResolution>>>::new();
    for rule in &pack.mappings {
        if rule.platform != platform {
            continue;
        }
        let Some(option) = native_options.get(&rule.native_capability_key) else {
            continue;
        };
        for native_choice in &option.choices {
            if let Some(semantic_choice) = rule.choices.get(&native_choice.value) {
                values
                    .entry(rule.semantic_facet.clone())
                    .or_default()
                    .insert(semantic_choice.clone());
                let candidate = SemanticNativeResolution {
                    native_option: rule.native_capability_key.clone(),
                    native_choice: native_choice.value.clone(),
                };
                resolutions
                    .entry(rule.semantic_facet.clone())
                    .or_default()
                    .entry(semantic_choice.clone())
                    .and_modify(|current| {
                        if current.as_ref() != Some(&candidate) {
                            *current = None;
                        }
                    })
                    .or_insert(Some(candidate));
            }
        }
    }
    SemanticPrinterCapabilities {
        facets: values
            .into_iter()
            .map(|(facet, choices)| (facet, choices.into_iter().collect()))
            .collect(),
        native_resolutions: resolutions
            .into_iter()
            .filter_map(|(facet, choices)| {
                let choices = choices
                    .into_iter()
                    .filter_map(|(choice, resolution)| resolution.map(|value| (choice, value)))
                    .collect::<BTreeMap<_, _>>();
                (!choices.is_empty()).then_some((facet, choices))
            })
            .collect(),
        support_pack: Some(SupportPackProvenance {
            pack_id: pack.manifest.pack_id.clone(),
            digest_sha256: hex::encode(pack.digest),
            evidence: evidence_name(pack.manifest.evidence).into(),
        }),
    }
}

fn parse_array<const N: usize>(value: &str, label: &str) -> Result<[u8; N], PackError> {
    let bytes = hex::decode(value).map_err(|_| PackError::Invalid(format!("invalid {label}")))?;
    bytes
        .try_into()
        .map_err(|_| PackError::Invalid(format!("invalid {label} length")))
}

fn to_printer_fingerprint(value: &DriverFingerprint) -> Option<PrinterFingerprint> {
    let platform = match value.platform.as_str() {
        "windows" => Platform::Windows,
        "macos" | "linux" | "cups" | "ipp" => Platform::CupsIpp,
        _ => return None,
    };
    let driver_package_sha256 = value.driver_package_fingerprint.as_ref()?.clone();
    validate_sha256(&driver_package_sha256).ok()?;
    let driver_version = value.driver_version.as_ref()?.clone();
    if value.driver_name.is_empty() || driver_version.is_empty() {
        return None;
    }
    Some(PrinterFingerprint {
        platform,
        driver_package_sha256,
        driver_id: value.driver_name.clone(),
        driver_version,
        device_id: value.device_fingerprint.clone(),
        firmware_version: value.firmware_version.clone(),
    })
}

const fn evidence_name(value: EvidenceTier) -> &'static str {
    match value {
        EvidenceTier::Discovered => "discovered",
        EvidenceTier::Mapped => "mapped",
        EvidenceTier::ReplayTested => "replay_tested",
        EvidenceTier::PhysicallyCertified => "physically_certified",
    }
}

/// Loads, verifies and validates a support-pack directory.
///
/// # Errors
///
/// Returns [`PackError`] when content is malformed, unsafe, untrusted or does
/// not pass its declared conformance cases.
pub fn load_pack(root: &Path, trust: &TrustPolicy) -> Result<LoadedPack, PackError> {
    let files = inventory(root)?;
    let digest = canonical_digest(root, &files)?;
    verify_trust(root, digest, trust)?;
    let manifest_path = root.join("manifest.json");
    let manifest: Manifest = read_json(&manifest_path)?;
    validate_manifest(&manifest)?;

    let mut mappings = Vec::new();
    for relative in &manifest.mappings {
        let path = safe_child(root, relative)?;
        let mapping: MappingFile = read_json(&path)?;
        if mapping.schema_version != 1 {
            return Err(PackError::Invalid(format!(
                "unsupported mapping schema in {relative}"
            )));
        }
        mappings.extend(mapping.rules);
    }
    if mappings.len() > MAX_RULES {
        return Err(PackError::Invalid("too many mapping rules".into()));
    }
    validate_mappings(&manifest, &mappings)?;
    validate_conformance(root, &manifest, &mappings)?;
    for relative in &manifest.fixtures {
        let path = safe_child(root, relative)?;
        let fixture: serde_json::Value = read_json(&path)?;
        validate_redacted_fixture(&fixture, 0)?;
    }
    Ok(LoadedPack {
        root: root.to_path_buf(),
        manifest,
        mappings,
        digest,
    })
}

/// Computes the canonical digest that an operator pins or a publisher signs.
///
/// # Errors
///
/// Returns [`PackError`] when the directory contains unsafe or unreadable
/// entries or exceeds the pack bounds.
pub fn pack_digest(root: &Path) -> Result<[u8; 32], PackError> {
    canonical_digest(root, &inventory(root)?)
}

/// Selects the sole trusted pack whose predicates exactly match a printer.
///
/// # Errors
///
/// Returns [`PackError::NoMatch`] or [`PackError::Ambiguous`] rather than
/// applying implicit precedence.
pub fn select_pack<'a>(
    packs: &'a [LoadedPack],
    printer: &PrinterFingerprint,
) -> Result<&'a LoadedPack, PackError> {
    let matching: Vec<_> = packs.iter().filter(|pack| matches(pack, printer)).collect();
    match matching.as_slice() {
        [one] => Ok(one),
        [] => Err(PackError::NoMatch),
        many => Err(PackError::Ambiguous(
            many.iter()
                .map(|pack| pack.manifest.pack_id.clone())
                .collect(),
        )),
    }
}

/// Selects the sole trusted pack whose exact predicates match an embedded
/// mobile adapter.
///
/// # Errors
///
/// Returns [`PackError::NoMatch`] or [`PackError::Ambiguous`].
pub fn select_adapter_pack<'a>(
    packs: &'a [LoadedPack],
    adapter: &AdapterFingerprint,
) -> Result<&'a LoadedPack, PackError> {
    let matching: Vec<_> = packs
        .iter()
        .filter(|pack| matches_adapter(pack, adapter))
        .collect();
    match matching.as_slice() {
        [one] => Ok(one),
        [] => Err(PackError::NoMatch),
        many => Err(PackError::Ambiguous(
            many.iter()
                .map(|pack| pack.manifest.pack_id.clone())
                .collect(),
        )),
    }
}

fn matches(pack: &LoadedPack, printer: &PrinterFingerprint) -> bool {
    pack.manifest.platforms.contains(&printer.platform)
        && pack.manifest.selectors.iter().any(|selector| {
            selector.driver_package_sha256 == printer.driver_package_sha256
                && selector.driver_id == printer.driver_id
                && selector.driver_version == printer.driver_version
                && selector
                    .device_id
                    .as_ref()
                    .is_none_or(|id| printer.device_id.as_ref() == Some(id))
                && selector
                    .firmware_version
                    .as_ref()
                    .is_none_or(|version| printer.firmware_version.as_ref() == Some(version))
        })
}

fn matches_adapter(pack: &LoadedPack, adapter: &AdapterFingerprint) -> bool {
    pack.manifest.platforms.contains(&adapter.platform)
        && pack.manifest.adapter_selectors.iter().any(|selector| {
            selector.platform == adapter.platform
                && selector.adapter_id == adapter.adapter_id
                && selector.adapter_version == adapter.adapter_version
                && selector
                    .device_family
                    .as_ref()
                    .is_none_or(|family| adapter.device_family.as_ref() == Some(family))
                && selector
                    .firmware_version
                    .as_ref()
                    .is_none_or(|version| adapter.firmware_version.as_ref() == Some(version))
        })
}

fn validate_manifest(manifest: &Manifest) -> Result<(), PackError> {
    if manifest.schema_version != 1 {
        return Err(PackError::Invalid("unsupported manifest schema".into()));
    }
    validate_id("pack_id", &manifest.pack_id, 128)?;
    validate_id("pack_version", &manifest.pack_version, 64)?;
    validate_id("vendor", &manifest.vendor, 128)?;
    validate_id("family", &manifest.family, 128)?;
    validate_id("license", &manifest.license, 128)?;
    if (manifest.selectors.is_empty() && manifest.adapter_selectors.is_empty())
        || manifest.platforms.is_empty()
        || manifest.maintainers.is_empty()
    {
        return Err(PackError::Invalid(
            "a driver or adapter selector, platforms and maintainers must not be empty".into(),
        ));
    }
    if manifest.mappings.is_empty() {
        return Err(PackError::Invalid(
            "at least one mapping is required".into(),
        ));
    }
    for selector in &manifest.selectors {
        validate_sha256(&selector.driver_package_sha256)?;
        if selector.driver_id.trim().is_empty() || selector.driver_version.trim().is_empty() {
            return Err(PackError::Invalid(
                "driver_id and driver_version must be exact non-empty values".into(),
            ));
        }
    }
    for selector in &manifest.adapter_selectors {
        if !manifest.platforms.contains(&selector.platform) {
            return Err(PackError::Invalid(
                "adapter selector declares an unlisted platform".into(),
            ));
        }
        validate_id("adapter_id", &selector.adapter_id, 256)?;
        validate_id("adapter_version", &selector.adapter_version, 64)?;
        if let Some(device_family) = &selector.device_family {
            validate_id("device_family", device_family, 128)?;
        }
        if let Some(firmware_version) = &selector.firmware_version {
            validate_id("firmware_version", firmware_version, 128)?;
        }
    }
    for maintainer in &manifest.maintainers {
        validate_id("maintainer", maintainer, 256)?;
    }
    for facet in &manifest.facets {
        validate_facet(facet)?;
    }
    Ok(())
}

fn validate_mappings(manifest: &Manifest, rules: &[MappingRule]) -> Result<(), PackError> {
    for rule in rules {
        validate_id("native_capability_key", &rule.native_capability_key, 256)?;
        validate_facet(&rule.semantic_facet)?;
        if !manifest.facets.contains(&rule.semantic_facet)
            || !manifest.platforms.contains(&rule.platform)
        {
            return Err(PackError::Invalid(
                "mapping declares an unlisted facet or platform".into(),
            ));
        }
        if rule.choices.is_empty() || rule.choices.len() > 128 {
            return Err(PackError::Invalid(
                "mapping choices must contain 1..=128 entries".into(),
            ));
        }
        for (native, semantic) in &rule.choices {
            validate_id("native choice", native, 256)?;
            validate_id("semantic choice", semantic, 128)?;
        }
    }
    Ok(())
}

fn validate_conformance(
    root: &Path,
    manifest: &Manifest,
    rules: &[MappingRule],
) -> Result<(), PackError> {
    if manifest.conformance.is_empty() {
        return Err(PackError::Invalid(
            "at least one conformance file is required".into(),
        ));
    }
    let mut has_positive = false;
    let mut has_unknown = false;
    for relative in &manifest.conformance {
        let file: ConformanceFile = read_json(&safe_child(root, relative)?)?;
        if file.schema_version != 1 || file.cases.is_empty() {
            return Err(PackError::Invalid(format!(
                "invalid conformance suite {relative}"
            )));
        }
        for case in file.cases {
            validate_id("conformance case name", &case.name, 256)?;
            let matched_rule = rules.iter().find(|rule| {
                rule.platform == case.platform
                    && rule.native_capability_key == case.native_capability_key
            });
            let resolved = matched_rule.and_then(|rule| {
                rule.choices
                    .get(&case.native_choice)
                    .map(|choice| (&rule.semantic_facet, choice))
            });
            match (matched_rule, resolved, case.expected_error.as_deref()) {
                (_, Some((facet, choice)), None)
                    if case.expected_semantic_facet.as_ref() == Some(facet)
                        && case.expected_semantic_choice.as_ref() == Some(choice) =>
                {
                    has_positive = true;
                }
                (Some(_), None, Some("unsupported_native_choice")) => has_unknown = true,
                _ => {
                    return Err(PackError::Invalid(format!(
                        "conformance case failed: {}",
                        case.name
                    )));
                }
            }
        }
    }
    if !has_positive || !has_unknown {
        return Err(PackError::Invalid(
            "conformance must cover a known and an unknown choice".into(),
        ));
    }
    Ok(())
}

fn validate_facet(value: &str) -> Result<(), PackError> {
    if value.starts_with("profile.safe_overrides")
        || !value.contains('.')
        || value.split('.').any(str::is_empty)
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
        })
    {
        return Err(PackError::Invalid(format!(
            "invalid or forbidden semantic facet: {value}"
        )));
    }
    validate_id("semantic facet", value, 128)
}

fn validate_id(label: &str, value: &str, max: usize) -> Result<(), PackError> {
    if value.is_empty()
        || value.len() > max
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:/+ -".contains(&b))
    {
        return Err(PackError::Invalid(format!("invalid {label}")));
    }
    Ok(())
}

fn validate_sha256(value: &str) -> Result<(), PackError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(PackError::Invalid(
            "driver_package_sha256 must be lowercase SHA-256 hex".into(),
        ));
    }
    Ok(())
}

fn validate_redacted_fixture(value: &serde_json::Value, depth: usize) -> Result<(), PackError> {
    if depth > MAX_FIXTURE_DEPTH {
        return Err(PackError::Invalid("fixture nesting is too deep".into()));
    }
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let lower = key.to_ascii_lowercase();
                if [
                    "serial",
                    "serial_number",
                    "bluetooth_address",
                    "mac_address",
                    "network_address",
                    "ip_address",
                    "api_key",
                    "token",
                    "secret",
                    "password",
                    "document",
                    "native_blob",
                    "devmode",
                ]
                .contains(&lower.as_str())
                {
                    return Err(PackError::Invalid(format!(
                        "fixture contains forbidden field {key}"
                    )));
                }
                validate_redacted_fixture(child, depth + 1)?;
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                validate_redacted_fixture(item, depth + 1)?;
            }
        }
        serde_json::Value::String(text) if text.len() > 4096 => {
            return Err(PackError::Invalid("fixture string is too long".into()));
        }
        _ => {}
    }
    Ok(())
}

fn verify_trust(root: &Path, digest: [u8; 32], trust: &TrustPolicy) -> Result<(), PackError> {
    if trust.pinned_digests.contains(&digest) {
        return Ok(());
    }
    let signature_path = root.join("SIGNATURE");
    let encoded = match fs::read_to_string(&signature_path) {
        Ok(encoded) => encoded,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(PackError::Untrusted);
        }
        Err(source) => {
            return Err(PackError::Io {
                path: signature_path,
                source,
            });
        }
    };
    let bytes = hex::decode(encoded.trim()).map_err(|_| PackError::InvalidSignature)?;
    let signature = Signature::from_slice(&bytes).map_err(|_| PackError::InvalidSignature)?;
    if trust
        .verifying_keys
        .iter()
        .any(|key| key.verify(&digest, &signature).is_ok())
    {
        Ok(())
    } else {
        Err(PackError::Untrusted)
    }
}

fn inventory(root: &Path) -> Result<Vec<PathBuf>, PackError> {
    fn walk(
        root: &Path,
        dir: &Path,
        output: &mut Vec<PathBuf>,
        total: &mut u64,
    ) -> Result<(), PackError> {
        for entry in fs::read_dir(dir).map_err(|source| PackError::Io {
            path: dir.to_path_buf(),
            source,
        })? {
            let entry = entry.map_err(|source| PackError::Io {
                path: dir.to_path_buf(),
                source,
            })?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|source| PackError::Io {
                path: path.clone(),
                source,
            })?;
            if metadata.file_type().is_symlink() {
                return Err(PackError::Invalid("symlinks are forbidden".into()));
            }
            if metadata.is_dir() {
                walk(root, &path, output, total)?;
            } else if metadata.is_file() {
                if metadata.len() > MAX_FILE_BYTES {
                    return Err(PackError::Invalid("pack file exceeds size limit".into()));
                }
                *total = total.saturating_add(metadata.len());
                if *total > MAX_PACK_BYTES {
                    return Err(PackError::Invalid("pack exceeds total size limit".into()));
                }
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| PackError::Invalid("file escapes pack root".into()))?
                    .to_path_buf();
                if relative.to_str().is_none() {
                    return Err(PackError::Invalid(
                        "pack paths must contain valid UTF-8".into(),
                    ));
                }
                if relative != Path::new("SIGNATURE") {
                    output.push(relative);
                }
                if output.len() > MAX_FILES {
                    return Err(PackError::Invalid("pack contains too many files".into()));
                }
            }
        }
        Ok(())
    }
    let root_metadata = fs::symlink_metadata(root).map_err(|source| PackError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(PackError::Invalid(
            "pack root must be a real directory, not a symlink".into(),
        ));
    }
    let mut files = Vec::new();
    walk(root, root, &mut files, &mut 0)?;
    files.sort();
    Ok(files)
}

fn canonical_digest(root: &Path, files: &[PathBuf]) -> Result<[u8; 32], PackError> {
    let mut hash = Sha256::new();
    for relative in files {
        let name = relative
            .to_str()
            .ok_or_else(|| PackError::Invalid("pack paths must contain valid UTF-8".into()))?;
        let contents = fs::read(root.join(relative)).map_err(|source| PackError::Io {
            path: root.join(relative),
            source,
        })?;
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        hash.update((contents.len() as u64).to_be_bytes());
        hash.update(contents);
    }
    Ok(hash.finalize().into())
}

fn safe_child(root: &Path, relative: &str) -> Result<PathBuf, PackError> {
    let path = Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(PackError::Invalid(format!("unsafe pack path: {relative}")));
    }
    Ok(root.join(path))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, PackError> {
    let bytes = fs::read(path).map_err(|source| PackError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|source| PackError::Json {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEST_PACK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestPack(PathBuf);

    impl TestPack {
        fn new() -> Result<Self, Box<dyn std::error::Error>> {
            let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
            let sequence = TEST_PACK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "piqae-support-pack-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(root.join("mappings"))?;
            fs::create_dir_all(root.join("fixtures"))?;
            fs::create_dir_all(root.join("tests"))?;
            fs::write(root.join("manifest.json"), manifest_json("pack.example"))?;
            fs::write(root.join("mappings/options.json"), mapping_json())?;
            fs::write(root.join("tests/conformance.json"), conformance_json())?;
            fs::write(
                root.join("fixtures/capabilities.redacted.json"),
                r#"{"model":"Example 100","options":["A","B"]}"#,
            )?;
            Ok(Self(root))
        }

        fn new_mobile() -> Result<Self, Box<dyn std::error::Error>> {
            let pack = Self::new()?;
            fs::write(pack.0.join("manifest.json"), mobile_manifest_json())?;
            fs::write(pack.0.join("mappings/options.json"), mobile_mapping_json())?;
            fs::write(
                pack.0.join("tests/conformance.json"),
                mobile_conformance_json(),
            )?;
            Ok(pack)
        }

        fn trust(&self) -> Result<TrustPolicy, PackError> {
            let digest = canonical_digest(&self.0, &inventory(&self.0)?)?;
            Ok(TrustPolicy {
                pinned_digests: BTreeSet::from([digest]),
                verifying_keys: Vec::new(),
            })
        }
    }

    impl Drop for TestPack {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn manifest_json(pack_id: &str) -> String {
        format!(
            r#"{{
          "schema_version":1,"pack_id":"{pack_id}","pack_version":"1.0.0",
          "vendor":"Example","family":"Example 100","maintainers":["Example Maintainer"],
          "selectors":[{{"driver_package_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","driver_id":"example-driver","driver_version":"1.2.3","device_id":"USBPRINT/example","firmware_version":"4.5"}}],
          "platforms":["windows"],"facets":["media.sensing"],"evidence":"mapped",
          "mappings":["mappings/options.json"],"conformance":["tests/conformance.json"],"fixtures":["fixtures/capabilities.redacted.json"],"license":"Apache-2.0"
        }}"#
        )
    }

    fn mapping_json() -> &'static str {
        r#"{"schema_version":1,"rules":[{"platform":"windows","native_capability_key":"display-safe-option","semantic_facet":"media.sensing","choices":{"Native A":"gap","Native B":"black_mark"}}]}"#
    }

    fn mobile_manifest_json() -> &'static str {
        r#"{
          "schema_version":1,"pack_id":"pack.example.mobile","pack_version":"1.0.0",
          "vendor":"Example","family":"Example Mobile","maintainers":["Example Maintainer"],
          "adapter_selectors":[{"platform":"ios_external_accessory","adapter_id":"com.example.print-adapter","adapter_version":"5.4.0","device_family":"Example Mobile","firmware_version":"4.5"}],
          "platforms":["ios_external_accessory"],"facets":["media.sensing"],"evidence":"mapped",
          "mappings":["mappings/options.json"],"conformance":["tests/conformance.json"],"fixtures":["fixtures/capabilities.redacted.json"],"license":"Apache-2.0"
        }"#
    }

    fn mobile_mapping_json() -> &'static str {
        r#"{"schema_version":1,"rules":[{"platform":"ios_external_accessory","native_capability_key":"display-safe-option","semantic_facet":"media.sensing","choices":{"Native A":"gap","Native B":"black_mark"}}]}"#
    }

    fn mobile_conformance_json() -> &'static str {
        r#"{"schema_version":1,"cases":[{"name":"maps","platform":"ios_external_accessory","native_capability_key":"display-safe-option","native_choice":"Native A","expected_semantic_facet":"media.sensing","expected_semantic_choice":"gap"},{"name":"unknown","platform":"ios_external_accessory","native_capability_key":"display-safe-option","native_choice":"Other","expected_error":"unsupported_native_choice"}]}"#
    }

    fn conformance_json() -> &'static str {
        r#"{"schema_version":1,"cases":[{"name":"maps","platform":"windows","native_capability_key":"display-safe-option","native_choice":"Native A","expected_semantic_facet":"media.sensing","expected_semantic_choice":"gap"},{"name":"unknown","platform":"windows","native_capability_key":"display-safe-option","native_choice":"Other","expected_error":"unsupported_native_choice"}]}"#
    }

    fn fingerprint() -> PrinterFingerprint {
        PrinterFingerprint {
            platform: Platform::Windows,
            driver_package_sha256: "a".repeat(64),
            driver_id: "example-driver".into(),
            driver_version: "1.2.3".into(),
            device_id: Some("USBPRINT/example".into()),
            firmware_version: Some("4.5".into()),
        }
    }

    fn adapter_fingerprint() -> AdapterFingerprint {
        AdapterFingerprint {
            platform: Platform::IosExternalAccessory,
            adapter_id: "com.example.print-adapter".into(),
            adapter_version: "5.4.0".into(),
            device_family: Some("Example Mobile".into()),
            firmware_version: Some("4.5".into()),
        }
    }

    #[test]
    fn loads_trusted_bounded_pack_and_matches_exactly() -> Result<(), Box<dyn std::error::Error>> {
        let pack = TestPack::new()?;
        let loaded = load_pack(&pack.0, &pack.trust()?)?;
        assert_eq!(
            select_pack(std::slice::from_ref(&loaded), &fingerprint())?
                .manifest
                .pack_id,
            "pack.example"
        );
        let mut wrong = fingerprint();
        wrong.driver_version = "1.2.4".into();
        assert!(matches!(
            select_pack(std::slice::from_ref(&loaded), &wrong),
            Err(PackError::NoMatch)
        ));
        Ok(())
    }

    #[test]
    fn verifies_detached_ed25519_signature() -> Result<(), Box<dyn std::error::Error>> {
        let pack = TestPack::new()?;
        let digest = pack_digest(&pack.0)?;
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        fs::write(
            pack.0.join("SIGNATURE"),
            hex::encode(signing_key.sign(&digest).to_bytes()),
        )?;
        let loaded = load_pack(
            &pack.0,
            &TrustPolicy {
                pinned_digests: BTreeSet::new(),
                verifying_keys: vec![signing_key.verifying_key()],
            },
        )?;
        assert_eq!(loaded.digest, digest);
        Ok(())
    }

    #[test]
    fn registry_projects_only_exact_proven_driver_fingerprints()
    -> Result<(), Box<dyn std::error::Error>> {
        let pack = TestPack::new()?;
        let digest = pack_digest(&pack.0)?;
        let config = RegistryConfig {
            pack_directories: vec![pack.0.clone()],
            pinned_digest_hex: vec![hex::encode(digest)],
            ed25519_public_key_hex: Vec::new(),
        };
        let registry = SupportPackRegistry::load(&config)?;
        let options = BTreeMap::from([(
            "display-safe-option".into(),
            NativePrinterOption {
                display_name: "Sensing".into(),
                default_choice: Some("Native A".into()),
                selected_choice: None,
                choices: vec![piqae_domain::NativePrinterChoice {
                    value: "Native A".into(),
                    display_name: "Native A".into(),
                }],
            },
        )]);
        let fingerprint = DriverFingerprint {
            platform: "windows".into(),
            driver_name: "example-driver".into(),
            driver_version: Some("1.2.3".into()),
            architecture: None,
            native_queue_id: "queue".into(),
            device_fingerprint: Some("USBPRINT/example".into()),
            driver_package_fingerprint: Some("a".repeat(64)),
            firmware_version: Some("4.5".into()),
        };
        let projection = registry.normalize(Some(&fingerprint), &options)?;
        assert_eq!(projection.facets["media.sensing"], ["gap"]);
        assert_eq!(
            projection.native_resolutions["media.sensing"]["gap"],
            SemanticNativeResolution {
                native_option: "display-safe-option".into(),
                native_choice: "Native A".into(),
            }
        );
        assert_eq!(
            projection
                .support_pack
                .as_ref()
                .map(|pack| pack.pack_id.as_str()),
            Some("pack.example")
        );

        let mut wrong_firmware = fingerprint.clone();
        wrong_firmware.firmware_version = Some("4.6".into());
        assert_eq!(
            registry.normalize(Some(&wrong_firmware), &options)?,
            SemanticPrinterCapabilities::default()
        );

        let mut incomplete = fingerprint;
        incomplete.driver_package_fingerprint = None;
        assert_eq!(
            registry.normalize(Some(&incomplete), &options)?,
            SemanticPrinterCapabilities::default()
        );
        // Reconstructing after a process restart produces the same projection.
        assert_eq!(
            SupportPackRegistry::load(&config)?.normalize(Some(&incomplete), &options)?,
            SemanticPrinterCapabilities::default()
        );
        Ok(())
    }

    #[test]
    fn registry_projects_only_exact_bundled_adapter_fingerprints()
    -> Result<(), Box<dyn std::error::Error>> {
        let pack = TestPack::new_mobile()?;
        let digest = pack_digest(&pack.0)?;
        let config = RegistryConfig {
            pack_directories: vec![pack.0.clone()],
            pinned_digest_hex: vec![hex::encode(digest)],
            ed25519_public_key_hex: Vec::new(),
        };
        let registry = SupportPackRegistry::load(&config)?;
        let options = BTreeMap::from([(
            "display-safe-option".into(),
            NativePrinterOption {
                display_name: "Sensing".into(),
                default_choice: Some("Native A".into()),
                selected_choice: None,
                choices: vec![piqae_domain::NativePrinterChoice {
                    value: "Native A".into(),
                    display_name: "Native A".into(),
                }],
            },
        )]);

        let projection = registry.normalize_adapter(&adapter_fingerprint(), &options)?;
        assert_eq!(projection.facets["media.sensing"], ["gap"]);
        assert_eq!(
            projection
                .support_pack
                .as_ref()
                .map(|pack| pack.pack_id.as_str()),
            Some("pack.example.mobile")
        );

        let mut wrong_version = adapter_fingerprint();
        wrong_version.adapter_version = "5.4.1".into();
        assert_eq!(
            registry.normalize_adapter(&wrong_version, &options)?,
            SemanticPrinterCapabilities::default()
        );
        let mut missing_firmware = adapter_fingerprint();
        missing_firmware.firmware_version = None;
        assert_eq!(
            registry.normalize_adapter(&missing_firmware, &options)?,
            SemanticPrinterCapabilities::default()
        );
        Ok(())
    }

    #[test]
    fn ambiguous_bundled_adapter_matches_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
        let first = TestPack::new_mobile()?;
        let second = TestPack::new_mobile()?;
        fs::write(
            second.0.join("manifest.json"),
            mobile_manifest_json().replace("pack.example.mobile", "pack.other.mobile"),
        )?;
        let packs = [
            load_pack(&first.0, &first.trust()?)?,
            load_pack(&second.0, &second.trust()?)?,
        ];
        assert!(
            matches!(select_adapter_pack(&packs, &adapter_fingerprint()), Err(PackError::Ambiguous(ids)) if ids.len() == 2)
        );
        Ok(())
    }

    #[test]
    fn checked_in_mobile_adapter_template_is_valid() -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../driver-support/templates/mobile-adapter");
        let digest = pack_digest(&root)?;
        let loaded = load_pack(
            &root,
            &TrustPolicy {
                pinned_digests: BTreeSet::from([digest]),
                verifying_keys: Vec::new(),
            },
        )?;
        assert_eq!(loaded.manifest.adapter_selectors.len(), 1);
        assert!(loaded.manifest.selectors.is_empty());
        Ok(())
    }

    #[test]
    fn rejects_untrusted_and_mutated_content() -> Result<(), Box<dyn std::error::Error>> {
        let pack = TestPack::new()?;
        let trust = pack.trust()?;
        fs::write(
            pack.0.join("mappings/options.json"),
            mapping_json().replace("gap", "continuous"),
        )?;
        assert!(matches!(
            load_pack(&pack.0, &trust),
            Err(PackError::Io { .. } | PackError::Untrusted)
        ));
        Ok(())
    }

    #[test]
    fn ambiguous_exact_matches_fail_closed() -> Result<(), Box<dyn std::error::Error>> {
        let first = TestPack::new()?;
        let second = TestPack::new()?;
        fs::write(second.0.join("manifest.json"), manifest_json("pack.other"))?;
        let packs = [
            load_pack(&first.0, &first.trust()?)?,
            load_pack(&second.0, &second.trust()?)?,
        ];
        assert!(
            matches!(select_pack(&packs, &fingerprint()), Err(PackError::Ambiguous(ids)) if ids.len() == 2)
        );
        Ok(())
    }

    #[test]
    fn rejects_sensitive_fixture_fields() -> Result<(), Box<dyn std::error::Error>> {
        for field in [
            "serial_number",
            "bluetooth_address",
            "mac_address",
            "network_address",
            "ip_address",
            "api_key",
        ] {
            let pack = TestPack::new()?;
            fs::write(
                pack.0.join("fixtures/capabilities.redacted.json"),
                format!(r#"{{"{field}":"customer-device"}}"#),
            )?;
            let trust = pack.trust()?;
            assert!(
                matches!(load_pack(&pack.0, &trust), Err(PackError::Invalid(message)) if message.contains(&format!("forbidden field {field}")))
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_safe_override_mapping_and_path_traversal() -> Result<(), Box<dyn std::error::Error>>
    {
        let pack = TestPack::new()?;
        fs::write(
            pack.0.join("mappings/options.json"),
            mapping_json().replace("media.sensing", "profile.safe_overrides.any"),
        )?;
        fs::write(
            pack.0.join("manifest.json"),
            manifest_json("pack.example").replace("media.sensing", "profile.safe_overrides.any"),
        )?;
        let trust = pack.trust()?;
        assert!(
            matches!(load_pack(&pack.0, &trust), Err(PackError::Invalid(message)) if message.contains("forbidden semantic facet"))
        );

        fs::write(
            pack.0.join("manifest.json"),
            manifest_json("pack.example").replace("mappings/options.json", "../outside.json"),
        )?;
        let trust = pack.trust()?;
        assert!(
            matches!(load_pack(&pack.0, &trust), Err(PackError::Invalid(message)) if message.contains("unsafe pack path"))
        );
        Ok(())
    }

    #[test]
    fn conformance_cases_match_platform_and_native_key() -> Result<(), Box<dyn std::error::Error>> {
        for case_name in ["maps", "unknown"] {
            let pack = TestPack::new()?;
            fs::write(
                pack.0.join("tests/conformance.json"),
                conformance_json().replace(
                    &format!(r#""name":"{case_name}","platform":"windows""#),
                    &format!(r#""name":"{case_name}","platform":"cups_ipp""#),
                ),
            )?;
            let trust = pack.trust()?;
            assert!(
                matches!(load_pack(&pack.0, &trust), Err(PackError::Invalid(message)) if message.contains(&format!("conformance case failed: {case_name}")))
            );
        }
        Ok(())
    }
}
