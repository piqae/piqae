use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const DEVMODE_SIZE_OFFSET: usize = 68;
const DEVMODE_DRIVER_EXTRA_OFFSET: usize = 70;
const DEVMODE_HEADER_END: usize = 72;
const MAX_DEVMODE_BYTES: usize = 16 * 1024 * 1024;
pub const WINDOWS_NATIVE_PROFILE_SCHEMA_VERSION: u16 = 1;
pub const WINDOWS_PROFILE_HOST_PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevmodeHeader {
    pub public_size: u16,
    pub private_size: u16,
    pub total_size: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpaqueDevmode {
    pub schema_version: u16,
    pub digest: String,
    pub bytes_base64: String,
    pub header: DevmodeHeader,
}

impl OpaqueDevmode {
    pub fn capture(bytes: &[u8]) -> Result<Self, NativeProfileError> {
        let header = validate_devmode_bytes(bytes)?;
        Ok(Self {
            schema_version: WINDOWS_NATIVE_PROFILE_SCHEMA_VERSION,
            digest: sha256_digest(bytes),
            bytes_base64: STANDARD.encode(bytes),
            header,
        })
    }

    pub fn decode_and_validate(&self) -> Result<Vec<u8>, NativeProfileError> {
        if self.schema_version != WINDOWS_NATIVE_PROFILE_SCHEMA_VERSION {
            return Err(NativeProfileError::new(
                "native_profile_schema_unsupported",
                format!(
                    "unsupported Windows native profile schema {}; expected {}",
                    self.schema_version, WINDOWS_NATIVE_PROFILE_SCHEMA_VERSION
                ),
            ));
        }
        let bytes = STANDARD.decode(&self.bytes_base64).map_err(|_| {
            NativeProfileError::new("devmode_base64_invalid", "invalid DEVMODE encoding")
        })?;
        let header = validate_devmode_bytes(&bytes)?;
        if header != self.header {
            return Err(NativeProfileError::new(
                "devmode_header_mismatch",
                "DEVMODE envelope metadata does not match the captured bytes",
            ));
        }
        let actual_digest = sha256_digest(&bytes);
        if !constant_time_equal(actual_digest.as_bytes(), self.digest.as_bytes()) {
            return Err(NativeProfileError::new(
                "devmode_digest_mismatch",
                "DEVMODE content digest does not match the captured bytes",
            ));
        }
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsDriverFingerprint {
    pub platform: String,
    pub driver_name: String,
    pub driver_version: String,
    pub driver_environment: String,
    pub architecture: String,
    pub native_queue_id: String,
    pub device_fingerprint: String,
    pub driver_date_filetime: Option<u64>,
}

impl WindowsDriverFingerprint {
    pub fn compatibility_error(&self, current: &Self) -> Option<NativeProfileError> {
        if self.platform != "windows" || current.platform != "windows" {
            return Some(NativeProfileError::new(
                "native_profile_platform_mismatch",
                "Windows DEVMODE profiles can only be used with a Windows driver",
            ));
        }
        if self.native_queue_id != current.native_queue_id {
            return Some(NativeProfileError::new(
                "destination_mismatch",
                "profile belongs to a different Windows printer queue",
            ));
        }
        if self.driver_name != current.driver_name
            || self.driver_version != current.driver_version
            || self.driver_environment != current.driver_environment
            || self.architecture != current.architecture
            || self.driver_date_filetime != current.driver_date_filetime
        {
            return Some(NativeProfileError::new(
                "driver_mismatch",
                format!(
                    "captured driver {} {} ({}) does not match installed driver {} {} ({})",
                    self.driver_name,
                    self.driver_version,
                    self.driver_environment,
                    current.driver_name,
                    current.driver_version,
                    current.driver_environment
                ),
            ));
        }
        if self.device_fingerprint != current.device_fingerprint {
            return Some(NativeProfileError::new(
                "device_mismatch",
                "printer port/device identity changed after profile capture",
            ));
        }
        None
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevmodeSummary {
    pub device_name: Option<String>,
    pub form_name: Option<String>,
    pub paper_size: Option<i16>,
    pub paper_width_tenths_mm: Option<i16>,
    pub paper_length_tenths_mm: Option<i16>,
    pub source: Option<i16>,
    pub copies: Option<i16>,
    pub print_quality: Option<i16>,
    pub color: Option<i16>,
    pub duplex: Option<i16>,
    pub y_resolution: Option<i16>,
    pub collate: Option<i16>,
    pub public_bytes: u16,
    pub private_bytes: u16,
}

impl DevmodeSummary {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, NativeProfileError> {
        let header = validate_devmode_bytes(bytes)?;
        let public = &bytes[..usize::from(header.public_size)];
        Ok(Self {
            device_name: utf16_field(public, 0, 32),
            form_name: utf16_field(public, 102, 32),
            paper_size: read_i16(public, 78),
            paper_length_tenths_mm: read_i16(public, 80),
            paper_width_tenths_mm: read_i16(public, 82),
            copies: read_i16(public, 86),
            source: read_i16(public, 88),
            print_quality: read_i16(public, 90),
            color: read_i16(public, 92),
            duplex: read_i16(public, 94),
            y_resolution: read_i16(public, 96),
            collate: read_i16(public, 100),
            public_bytes: header.public_size,
            private_bytes: header.private_size,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsNativeProfileCapture {
    pub kind: String,
    pub schema_version: u16,
    pub fingerprint: WindowsDriverFingerprint,
    pub devmode: OpaqueDevmode,
    pub summary: DevmodeSummary,
}

impl WindowsNativeProfileCapture {
    pub fn new(
        fingerprint: WindowsDriverFingerprint,
        devmode_bytes: &[u8],
    ) -> Result<Self, NativeProfileError> {
        Ok(Self {
            kind: "windows_devmode".into(),
            schema_version: WINDOWS_NATIVE_PROFILE_SCHEMA_VERSION,
            fingerprint,
            devmode: OpaqueDevmode::capture(devmode_bytes)?,
            summary: DevmodeSummary::from_bytes(devmode_bytes)?,
        })
    }

    pub fn validate_envelope(&self) -> Result<Vec<u8>, NativeProfileError> {
        if self.kind != "windows_devmode" {
            return Err(NativeProfileError::new(
                "native_profile_kind_unsupported",
                format!("unsupported Windows native profile kind {}", self.kind),
            ));
        }
        if self.schema_version != WINDOWS_NATIVE_PROFILE_SCHEMA_VERSION {
            return Err(NativeProfileError::new(
                "native_profile_schema_unsupported",
                format!("unsupported Windows profile schema {}", self.schema_version),
            ));
        }
        let bytes = self.devmode.decode_and_validate()?;
        if DevmodeSummary::from_bytes(&bytes)? != self.summary {
            return Err(NativeProfileError::new(
                "devmode_summary_mismatch",
                "DEVMODE public summary does not match the captured bytes",
            ));
        }
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileHostRequest {
    pub protocol_version: u16,
    pub request_id: String,
    pub capture_token: String,
    pub operation: ProfileHostOperation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProfileHostOperation {
    Capture {
        native_printer_id: String,
        owner_window: Option<isize>,
        existing: Option<WindowsNativeProfileCapture>,
    },
    Validate {
        native_printer_id: String,
        capture: WindowsNativeProfileCapture,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileHostResponse {
    pub protocol_version: u16,
    pub request_id: String,
    pub result: Result<ProfileHostResult, NativeProfileError>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProfileHostResult {
    Captured {
        capture: WindowsNativeProfileCapture,
    },
    Validated {
        capture: WindowsNativeProfileCapture,
    },
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeProfileError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl NativeProfileError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            retryable: false,
        }
    }
}

impl std::fmt::Display for NativeProfileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for NativeProfileError {}

pub fn validate_devmode_bytes(bytes: &[u8]) -> Result<DevmodeHeader, NativeProfileError> {
    if bytes.len() < DEVMODE_HEADER_END {
        return Err(NativeProfileError::new(
            "devmode_truncated",
            "DEVMODE does not contain its public size header",
        ));
    }
    if bytes.len() > MAX_DEVMODE_BYTES {
        return Err(NativeProfileError::new(
            "devmode_too_large",
            "DEVMODE exceeds the 16 MiB native profile limit",
        ));
    }
    let public_size =
        u16::from_le_bytes([bytes[DEVMODE_SIZE_OFFSET], bytes[DEVMODE_SIZE_OFFSET + 1]]);
    let private_size = u16::from_le_bytes([
        bytes[DEVMODE_DRIVER_EXTRA_OFFSET],
        bytes[DEVMODE_DRIVER_EXTRA_OFFSET + 1],
    ]);
    if usize::from(public_size) < DEVMODE_HEADER_END {
        return Err(NativeProfileError::new(
            "devmode_public_size_invalid",
            format!("DEVMODE public size {public_size} is smaller than its required header"),
        ));
    }
    let total_size = usize::from(public_size)
        .checked_add(usize::from(private_size))
        .ok_or_else(|| NativeProfileError::new("devmode_size_overflow", "DEVMODE size overflow"))?;
    if total_size != bytes.len() {
        return Err(NativeProfileError::new(
            "devmode_size_mismatch",
            format!(
                "DEVMODE declares {total_size} bytes but the capture contains {}",
                bytes.len()
            ),
        ));
    }
    Ok(DevmodeHeader {
        public_size,
        private_size,
        total_size,
    })
}

fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn read_i16(bytes: &[u8], offset: usize) -> Option<i16> {
    let raw = bytes.get(offset..offset.checked_add(2)?)?;
    Some(i16::from_le_bytes([raw[0], raw[1]]))
}

fn utf16_field(bytes: &[u8], offset: usize, units: usize) -> Option<String> {
    let end = offset.checked_add(units.checked_mul(2)?)?;
    let raw = bytes.get(offset..end)?;
    let values = raw
        .chunks_exact(2)
        .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
        .take_while(|unit| *unit != 0)
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| String::from_utf16_lossy(&values))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(public: u16, private: u16) -> Vec<u8> {
        let mut bytes = vec![0_u8; usize::from(public) + usize::from(private)];
        bytes[DEVMODE_SIZE_OFFSET..DEVMODE_SIZE_OFFSET + 2].copy_from_slice(&public.to_le_bytes());
        bytes[DEVMODE_DRIVER_EXTRA_OFFSET..DEVMODE_DRIVER_EXTRA_OFFSET + 2]
            .copy_from_slice(&private.to_le_bytes());
        for (offset, unit) in "Fixture printer".encode_utf16().enumerate() {
            bytes[offset * 2..offset * 2 + 2].copy_from_slice(&unit.to_le_bytes());
        }
        if public >= 166 {
            for (index, unit) in "A4".encode_utf16().enumerate() {
                let offset = 102 + index * 2;
                bytes[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
            }
        }
        bytes
    }

    fn fingerprint() -> WindowsDriverFingerprint {
        WindowsDriverFingerprint {
            platform: "windows".into(),
            driver_name: "Fixture Driver".into(),
            driver_version: "1.2.3.4".into(),
            driver_environment: "Windows x64".into(),
            architecture: "x86_64".into(),
            native_queue_id: "Fixture".into(),
            device_fingerprint: "sha256:fixture".into(),
            driver_date_filetime: Some(42),
        }
    }

    #[test]
    fn opaque_capture_preserves_private_driver_bytes() {
        let mut bytes = fixture(220, 128);
        bytes[220..].fill(0xa5);
        let capture = WindowsNativeProfileCapture::new(fingerprint(), &bytes).expect("capture");
        assert_eq!(capture.devmode.header.private_size, 128);
        assert_eq!(capture.validate_envelope().expect("valid"), bytes);
        assert_eq!(
            capture.summary.device_name.as_deref(),
            Some("Fixture printer")
        );
        assert_eq!(capture.summary.form_name.as_deref(), Some("A4"));
    }

    #[test]
    fn altered_private_bytes_fail_digest_validation() {
        let bytes = fixture(220, 64);
        let mut capture = WindowsNativeProfileCapture::new(fingerprint(), &bytes).expect("capture");
        let mut decoded = STANDARD
            .decode(&capture.devmode.bytes_base64)
            .expect("decode");
        decoded[230] ^= 1;
        capture.devmode.bytes_base64 = STANDARD.encode(decoded);
        let error = capture.validate_envelope().expect_err("tampering rejected");
        assert_eq!(error.code, "devmode_digest_mismatch");
    }

    #[test]
    fn truncated_and_mismatched_buffers_are_rejected() {
        let short = vec![0_u8; 71];
        assert_eq!(
            validate_devmode_bytes(&short).expect_err("short").code,
            "devmode_truncated"
        );
        let mut mismatched = fixture(220, 32);
        mismatched.push(0);
        assert_eq!(
            validate_devmode_bytes(&mismatched)
                .expect_err("mismatch")
                .code,
            "devmode_size_mismatch"
        );
    }

    #[test]
    fn profile_host_protocol_round_trips_without_losing_blob_bytes() {
        let bytes = fixture(220, 32);
        let request = ProfileHostRequest {
            protocol_version: WINDOWS_PROFILE_HOST_PROTOCOL_VERSION,
            request_id: "req_01".into(),
            capture_token: "a".repeat(32),
            operation: ProfileHostOperation::Validate {
                native_printer_id: "Fixture".into(),
                capture: WindowsNativeProfileCapture::new(fingerprint(), &bytes).expect("capture"),
            },
        };
        let encoded = serde_json::to_vec(&request).expect("serialize");
        let decoded: ProfileHostRequest = serde_json::from_slice(&encoded).expect("deserialize");
        assert_eq!(decoded, request);
    }

    #[test]
    fn compatibility_reports_queue_driver_and_device_changes() {
        let expected = fingerprint();
        let mut current = expected.clone();
        current.driver_version = "2.0.0.0".into();
        assert_eq!(
            expected
                .compatibility_error(&current)
                .expect("mismatch")
                .code,
            "driver_mismatch"
        );
        current = expected.clone();
        current.device_fingerprint = "sha256:other".into();
        assert_eq!(
            expected
                .compatibility_error(&current)
                .expect("mismatch")
                .code,
            "device_mismatch"
        );
    }
}
