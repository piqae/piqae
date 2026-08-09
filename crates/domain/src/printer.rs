use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

pub const PRINTER_PROFILE_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrinterState {
    Online,
    Offline,
    Paused,
    Busy,
    PaperOut,
    Error,
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors independent driver capability flags"
)]
pub struct PrinterCapabilities {
    pub bins: Vec<String>,
    pub collate: bool,
    pub color: bool,
    pub copies: u32,
    pub dpis: Vec<String>,
    pub duplex: bool,
    pub extent: Vec<[u32; 2]>,
    pub medias: Vec<String>,
    pub nup: Vec<u16>,
    pub papers: BTreeMap<String, [Option<u32>; 2]>,
    pub printrate: Option<PrintRate>,
    pub supports_custom_paper_size: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PrinterCapabilityProfile {
    pub schema_version: u16,
    pub revision: u64,
    pub portable: PrinterCapabilities,
    #[serde(default)]
    pub native_options: BTreeMap<String, NativePrinterOption>,
}

impl PrinterCapabilityProfile {
    #[must_use]
    pub const fn draft(
        portable: PrinterCapabilities,
        native_options: BTreeMap<String, NativePrinterOption>,
    ) -> Self {
        Self {
            schema_version: PRINTER_PROFILE_SCHEMA_VERSION,
            revision: 0,
            portable,
            native_options,
        }
    }

    /// Validates the stable portable/native profile contract.
    ///
    /// # Errors
    ///
    /// Returns a descriptive error for an unsupported schema, malformed
    /// option key, duplicate choice, or a default absent from its choices.
    pub fn validate(&self) -> Result<(), PrinterProfileError> {
        if self.schema_version != PRINTER_PROFILE_SCHEMA_VERSION {
            return Err(PrinterProfileError::UnsupportedSchema(self.schema_version));
        }
        for (key, option) in &self.native_options {
            if key.trim().is_empty() {
                return Err(PrinterProfileError::EmptyOptionKey);
            }
            if option.display_name.trim().is_empty() {
                return Err(PrinterProfileError::EmptyOptionName(key.clone()));
            }
            let mut choices = std::collections::BTreeSet::new();
            for choice in &option.choices {
                if choice.value.trim().is_empty() || !choices.insert(choice.value.as_str()) {
                    return Err(PrinterProfileError::InvalidChoice(key.clone()));
                }
            }
            if let Some(default) = &option.default_choice
                && !option.choices.is_empty()
                && !choices.contains(default.as_str())
            {
                return Err(PrinterProfileError::UnknownDefault(key.clone()));
            }
            if let Some(selected) = &option.selected_choice
                && !option.choices.is_empty()
                && !choices.contains(selected.as_str())
            {
                return Err(PrinterProfileError::UnknownSelection(key.clone()));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativePrinterOption {
    pub display_name: String,
    pub default_choice: Option<String>,
    #[serde(default)]
    pub selected_choice: Option<String>,
    pub choices: Vec<NativePrinterChoice>,
}

/// Display-safe provenance for semantic capabilities normalized by one exact,
/// trusted driver support pack.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SupportPackProvenance {
    pub pack_id: String,
    pub digest_sha256: String,
    pub evidence: String,
}

/// Normalized, display-safe choices. This never carries native execution
/// values or changes the overrides authorized by an immutable profile.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct SemanticPrinterCapabilities {
    pub facets: BTreeMap<String, Vec<String>>,
    pub support_pack: Option<SupportPackProvenance>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativePrinterChoice {
    pub value: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum PrinterProfileError {
    #[error("printer profile schema {0} is unsupported")]
    UnsupportedSchema(u16),
    #[error("native printer option key cannot be empty")]
    EmptyOptionKey,
    #[error("native printer option {0} has an empty display name")]
    EmptyOptionName(String),
    #[error("native printer option {0} has an empty or duplicate choice")]
    InvalidChoice(String),
    #[error("native printer option {0} has a default absent from its choices")]
    UnknownDefault(String),
    #[error("native printer option {0} has a selection absent from its choices")]
    UnknownSelection(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PrintRate {
    pub unit: PrintRateUnit,
    pub rate: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PrintRateUnit {
    Ppm,
    Ipm,
    Lmp,
    Cpm,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_rejects_a_default_absent_from_native_choices() {
        let mut options = BTreeMap::new();
        options.insert(
            "Duplex".into(),
            NativePrinterOption {
                display_name: "Two-sided".into(),
                default_choice: Some("DuplexNoTumble".into()),
                selected_choice: Some("DuplexNoTumble".into()),
                choices: vec![NativePrinterChoice {
                    value: "None".into(),
                    display_name: "Off".into(),
                }],
            },
        );
        assert_eq!(
            PrinterCapabilityProfile::draft(PrinterCapabilities::default(), options).validate(),
            Err(PrinterProfileError::UnknownDefault("Duplex".into()))
        );
    }
}
