use crate::native_profile::{
    NativeProfileError, WindowsDriverFingerprint, WindowsNativeProfileCapture,
};
use piqae_domain::{Duplex, JobOptions, SafeProfileOverride};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

const DEVMODE_FIELDS_OFFSET: usize = 72;
const DEVMODE_PAPER_SIZE_OFFSET: usize = 78;
const DEVMODE_COPIES_OFFSET: usize = 86;
const DEVMODE_DEFAULT_SOURCE_OFFSET: usize = 88;
const DEVMODE_PRINT_QUALITY_OFFSET: usize = 90;
const DEVMODE_COLOR_OFFSET: usize = 92;
const DEVMODE_DUPLEX_OFFSET: usize = 94;
const DEVMODE_Y_RESOLUTION_OFFSET: usize = 96;
const DEVMODE_COLLATE_OFFSET: usize = 100;
const DEVMODE_FORM_NAME_OFFSET: usize = 102;
const DEVMODE_FORM_NAME_UNITS: usize = 32;

const DM_PAPER_SIZE: u32 = 0x0000_0002;
const DM_COPIES: u32 = 0x0000_0100;
const DM_DEFAULT_SOURCE: u32 = 0x0000_0200;
const DM_PRINT_QUALITY: u32 = 0x0000_0400;
const DM_COLOR: u32 = 0x0000_0800;
const DM_DUPLEX: u32 = 0x0000_1000;
const DM_Y_RESOLUTION: u32 = 0x0000_2000;
const DM_COLLATE: u32 = 0x0000_8000;
const DM_FORM_NAME: u32 = 0x0001_0000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsPdfBackend {
    /// PDFium rasterization into a GDI printer device context.
    GdiPdfium,
    /// Generic preview compatibility path. It cannot replay native profiles.
    SumatraPreview,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileReplayPlan {
    /// Structurally validated captured DEVMODE, including opaque driver bytes.
    pub devmode_bytes: Vec<u8>,
    pub requested_overrides: BTreeSet<String>,
}

/// Validates the immutable envelope, installed-driver identity and per-job
/// override allow-list before any native handoff.
pub fn prepare_profile_replay(
    capture: &WindowsNativeProfileCapture,
    current_fingerprint: &WindowsDriverFingerprint,
    options: &JobOptions,
    safe_overrides: &BTreeSet<String>,
    backend: WindowsPdfBackend,
) -> Result<ProfileReplayPlan, NativeProfileError> {
    let devmode_bytes = capture.validate_envelope()?;
    if let Some(error) = capture.fingerprint.compatibility_error(current_fingerprint) {
        return Err(error);
    }
    let requested = requested_overrides(options);
    if let Some(disallowed) = requested.difference(safe_overrides).next() {
        return Err(NativeProfileError::new(
            "profile_override_not_allowed",
            format!(
                "profile does not allow {disallowed} to be changed per job; allowed overrides: {}",
                safe_overrides
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }
    if backend == WindowsPdfBackend::SumatraPreview {
        return Err(NativeProfileError::new(
            "native_profile_backend_unavailable",
            "Sumatra preview printing cannot replay a captured Windows DEVMODE profile",
        ));
    }
    Ok(ProfileReplayPlan {
        devmode_bytes,
        requested_overrides: requested,
    })
}

pub fn safe_override_names(overrides: &[SafeProfileOverride]) -> BTreeSet<String> {
    overrides
        .iter()
        .map(|value| match value {
            SafeProfileOverride::Bin => "bin",
            SafeProfileOverride::Collate => "collate",
            SafeProfileOverride::Color => "color",
            SafeProfileOverride::Copies => "copies",
            SafeProfileOverride::Dpi => "dpi",
            SafeProfileOverride::Duplex => "duplex",
            SafeProfileOverride::FitToPage => "fit_to_page",
            SafeProfileOverride::Media => "media",
            SafeProfileOverride::Nup => "nup",
            SafeProfileOverride::Pages => "pages",
            SafeProfileOverride::Paper => "paper",
            SafeProfileOverride::Rotate => "rotate",
        })
        .map(str::to_owned)
        .collect()
}

/// Applies only documented public DEVMODE fields. The driver's opaque
/// `dmDriverExtra` bytes are deliberately left untouched and the complete
/// result must be normalized with `DocumentPropertiesW` before `CreateDCW`.
pub fn apply_public_devmode_overrides(
    bytes: &mut [u8],
    options: &JobOptions,
) -> Result<(), NativeProfileError> {
    let header = crate::native_profile::validate_devmode_bytes(bytes)?;
    let public = bytes
        .get_mut(..usize::from(header.public_size))
        .ok_or_else(|| NativeProfileError::new("devmode_truncated", "DEVMODE is truncated"))?;
    let mut fields = read_u32(public, DEVMODE_FIELDS_OFFSET)?;

    if let Some(copies) = options.copies {
        let copies = i16::try_from(copies).map_err(|_| {
            NativeProfileError::new(
                "windows_copies_invalid",
                "Windows DEVMODE copies must be between 1 and 32767",
            )
        })?;
        if copies < 1 {
            return Err(NativeProfileError::new(
                "windows_copies_invalid",
                "Windows DEVMODE copies must be between 1 and 32767",
            ));
        }
        write_i16(public, DEVMODE_COPIES_OFFSET, copies)?;
        fields |= DM_COPIES;
    }
    if let Some(color) = options.color {
        write_i16(public, DEVMODE_COLOR_OFFSET, if color { 2 } else { 1 })?;
        fields |= DM_COLOR;
    }
    if let Some(collate) = options.collate {
        write_i16(public, DEVMODE_COLLATE_OFFSET, if collate { 1 } else { 0 })?;
        fields |= DM_COLLATE;
    }
    if let Some(duplex) = options.duplex {
        let value = match duplex {
            Duplex::OneSided => 1,
            Duplex::LongEdge => 2,
            Duplex::ShortEdge => 3,
        };
        write_i16(public, DEVMODE_DUPLEX_OFFSET, value)?;
        fields |= DM_DUPLEX;
    }
    if let Some(dpi) = options.dpi.as_deref() {
        let (horizontal, vertical) = parse_dpi(dpi)?;
        write_i16(public, DEVMODE_PRINT_QUALITY_OFFSET, horizontal)?;
        write_i16(public, DEVMODE_Y_RESOLUTION_OFFSET, vertical)?;
        fields |= DM_PRINT_QUALITY | DM_Y_RESOLUTION;
    }
    if let Some(bin) = options.bin.as_deref() {
        let source = parse_positive_i16(bin, "windows_bin_invalid")?;
        write_i16(public, DEVMODE_DEFAULT_SOURCE_OFFSET, source)?;
        fields |= DM_DEFAULT_SOURCE;
    }
    if let Some(paper) = options.paper.as_deref() {
        if let Ok(paper_id) = paper.parse::<i16>() {
            if paper_id <= 0 {
                return Err(NativeProfileError::new(
                    "windows_paper_invalid",
                    "Windows paper ID must be a positive integer",
                ));
            }
            write_i16(public, DEVMODE_PAPER_SIZE_OFFSET, paper_id)?;
            fields |= DM_PAPER_SIZE;
        } else {
            write_utf16_field(
                public,
                DEVMODE_FORM_NAME_OFFSET,
                DEVMODE_FORM_NAME_UNITS,
                paper,
            )?;
            fields = (fields | DM_FORM_NAME) & !DM_PAPER_SIZE;
        }
    }
    // Rotation and page ranges alter PDF content, not the captured paper
    // geometry, so they are applied by the renderer rather than DEVMODE.
    if options.media.is_some() || options.nup.is_some() {
        return Err(NativeProfileError::new(
            "windows_profile_override_unsupported",
            "Windows native replay does not assign portable media or n-up values; capture them in the vendor driver profile",
        ));
    }
    write_u32(public, DEVMODE_FIELDS_OFFSET, fields)
}

#[must_use]
pub fn profile_blob_digest(blob: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(blob)))
}

pub fn selected_pages(
    specification: Option<&str>,
    count: usize,
) -> Result<Vec<usize>, NativeProfileError> {
    if count == 0 {
        return Err(NativeProfileError::new(
            "pdf_has_no_pages",
            "PDF contains no printable pages",
        ));
    }
    let Some(specification) = specification else {
        return Ok((0..count).collect());
    };
    let mut selected = BTreeSet::new();
    for segment in specification.split(',').map(str::trim) {
        if segment.is_empty() {
            return Err(invalid_page_range());
        }
        let mut bounds = segment.split('-').map(str::trim);
        let start = parse_page_number(bounds.next().unwrap_or_default(), count)?;
        let end = bounds
            .next()
            .map_or(Ok(start), |value| parse_page_number(value, count))?;
        if bounds.next().is_some() || end < start {
            return Err(invalid_page_range());
        }
        selected.extend((start - 1)..end);
    }
    if selected.is_empty() {
        return Err(invalid_page_range());
    }
    Ok(selected.into_iter().collect())
}

fn requested_overrides(options: &JobOptions) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    if options.bin.is_some() {
        names.insert("bin".into());
    }
    if options.collate.is_some() {
        names.insert("collate".into());
    }
    if options.color.is_some() {
        names.insert("color".into());
    }
    if options.copies.is_some() {
        names.insert("copies".into());
    }
    if options.dpi.is_some() {
        names.insert("dpi".into());
    }
    if options.duplex.is_some() {
        names.insert("duplex".into());
    }
    if options.fit_to_page.is_some() {
        names.insert("fit_to_page".into());
    }
    if options.media.is_some() {
        names.insert("media".into());
    }
    if options.nup.is_some() {
        names.insert("nup".into());
    }
    if options.pages.is_some() {
        names.insert("pages".into());
    }
    if options.paper.is_some() {
        names.insert("paper".into());
    }
    if options.rotate.is_some() {
        names.insert("rotate".into());
    }
    names
}

fn parse_page_number(value: &str, count: usize) -> Result<usize, NativeProfileError> {
    value
        .parse::<usize>()
        .ok()
        .filter(|page| *page > 0 && *page <= count)
        .ok_or_else(invalid_page_range)
}

fn invalid_page_range() -> NativeProfileError {
    NativeProfileError::new(
        "pdf_page_range_invalid",
        "page range must use one-based values such as 1,3-5 within the PDF",
    )
}

fn parse_dpi(value: &str) -> Result<(i16, i16), NativeProfileError> {
    let normalized = value.trim().to_ascii_lowercase();
    let mut parts = normalized.split('x');
    let horizontal = parts.next().unwrap_or_default();
    let vertical = parts.next();
    if parts.next().is_some() {
        return Err(NativeProfileError::new(
            "windows_dpi_invalid",
            "DPI must be a positive integer or WIDTHxHEIGHT",
        ));
    }
    let horizontal = parse_positive_i16(horizontal, "windows_dpi_invalid")?;
    let vertical = vertical.map_or(Ok(horizontal), |part| {
        parse_positive_i16(part, "windows_dpi_invalid")
    })?;
    Ok((horizontal, vertical))
}

fn parse_positive_i16(value: &str, code: &'static str) -> Result<i16, NativeProfileError> {
    value
        .trim()
        .parse::<i16>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            NativeProfileError::new(
                code,
                "value must be a positive integer no greater than 32767",
            )
        })
}

#[cfg(test)]
fn read_i16(bytes: &[u8], offset: usize) -> Result<i16, NativeProfileError> {
    let raw = bytes.get(offset..offset.saturating_add(2)).ok_or_else(|| {
        NativeProfileError::new("devmode_truncated", "DEVMODE public fields are truncated")
    })?;
    Ok(i16::from_le_bytes([raw[0], raw[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, NativeProfileError> {
    let raw = bytes.get(offset..offset.saturating_add(4)).ok_or_else(|| {
        NativeProfileError::new("devmode_truncated", "DEVMODE public fields are truncated")
    })?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn write_i16(bytes: &mut [u8], offset: usize, value: i16) -> Result<(), NativeProfileError> {
    let target = bytes
        .get_mut(offset..offset.saturating_add(2))
        .ok_or_else(|| {
            NativeProfileError::new("devmode_truncated", "DEVMODE public fields are truncated")
        })?;
    target.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) -> Result<(), NativeProfileError> {
    let target = bytes
        .get_mut(offset..offset.saturating_add(4))
        .ok_or_else(|| {
            NativeProfileError::new("devmode_truncated", "DEVMODE public fields are truncated")
        })?;
    target.copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn write_utf16_field(
    bytes: &mut [u8],
    offset: usize,
    units: usize,
    value: &str,
) -> Result<(), NativeProfileError> {
    let encoded = value.encode_utf16().collect::<Vec<_>>();
    if encoded.is_empty() || encoded.len() >= units {
        return Err(NativeProfileError::new(
            "windows_paper_invalid",
            format!(
                "Windows form name must contain between 1 and {} UTF-16 code units",
                units - 1
            ),
        ));
    }
    let target = bytes
        .get_mut(offset..offset.saturating_add(units.saturating_mul(2)))
        .ok_or_else(|| {
            NativeProfileError::new("devmode_truncated", "DEVMODE form-name field is truncated")
        })?;
    target.fill(0);
    for (index, unit) in encoded.into_iter().enumerate() {
        target[index * 2..index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_profile::WindowsDriverFingerprint;

    fn fixture() -> WindowsNativeProfileCapture {
        let mut bytes = vec![0_u8; 228];
        bytes[68..70].copy_from_slice(&220_u16.to_le_bytes());
        bytes[70..72].copy_from_slice(&8_u16.to_le_bytes());
        WindowsNativeProfileCapture::new(
            WindowsDriverFingerprint {
                platform: "windows".into(),
                driver_name: "Driver".into(),
                driver_version: "1.0.0.0".into(),
                driver_environment: "Windows x64".into(),
                architecture: "x86_64".into(),
                native_queue_id: "Printer".into(),
                device_fingerprint: "sha256:fixture".into(),
                driver_date_filetime: None,
            },
            &bytes,
        )
        .expect("fixture")
    }

    #[test]
    fn permits_only_explicit_safe_overrides() {
        let capture = fixture();
        let options = JobOptions {
            copies: Some(2),
            pages: Some("1".into()),
            ..Default::default()
        };
        let safe = BTreeSet::from(["copies".into(), "pages".into()]);
        let plan = prepare_profile_replay(
            &capture,
            &capture.fingerprint,
            &options,
            &safe,
            WindowsPdfBackend::GdiPdfium,
        )
        .expect("plan");
        assert_eq!(plan.requested_overrides, safe);
    }

    #[test]
    fn rejects_unsafe_overrides_before_native_handoff() {
        let capture = fixture();
        let options = JobOptions {
            paper: Some("Letter".into()),
            ..Default::default()
        };
        let error = prepare_profile_replay(
            &capture,
            &capture.fingerprint,
            &options,
            &BTreeSet::from(["copies".into()]),
            WindowsPdfBackend::GdiPdfium,
        )
        .expect_err("unsafe");
        assert_eq!(error.code, "profile_override_not_allowed");
    }

    #[test]
    fn sumatra_is_never_treated_as_native_profile_replay() {
        let capture = fixture();
        let error = prepare_profile_replay(
            &capture,
            &capture.fingerprint,
            &JobOptions::default(),
            &BTreeSet::new(),
            WindowsPdfBackend::SumatraPreview,
        )
        .expect_err("unsupported backend");
        assert_eq!(error.code, "native_profile_backend_unavailable");
    }

    #[test]
    fn public_overrides_preserve_opaque_driver_bytes() {
        let capture = fixture();
        let mut bytes = capture.validate_envelope().expect("capture");
        bytes[220..].fill(0xa5);
        let private = bytes[220..].to_vec();
        let options = JobOptions {
            copies: Some(3),
            color: Some(false),
            collate: Some(true),
            duplex: Some(piqae_domain::Duplex::ShortEdge),
            dpi: Some("600x300".into()),
            paper: Some("A4 Borderless".into()),
            ..Default::default()
        };
        apply_public_devmode_overrides(&mut bytes, &options).expect("overrides");
        assert_eq!(&bytes[220..], private);
        assert_eq!(read_i16(&bytes, DEVMODE_COPIES_OFFSET).expect("copies"), 3);
        assert_eq!(
            read_i16(&bytes, DEVMODE_PRINT_QUALITY_OFFSET).expect("dpi"),
            600
        );
        assert_eq!(
            read_i16(&bytes, DEVMODE_Y_RESOLUTION_OFFSET).expect("dpi"),
            300
        );
        assert_ne!(
            read_u32(&bytes, DEVMODE_FIELDS_OFFSET).expect("fields") & DM_FORM_NAME,
            0
        );
    }

    #[test]
    fn unsupported_portable_semantics_fail_closed() {
        let capture = fixture();
        let mut bytes = capture.validate_envelope().expect("capture");
        let error = apply_public_devmode_overrides(
            &mut bytes,
            &JobOptions {
                media: Some("glossy".into()),
                ..Default::default()
            },
        )
        .expect_err("unsupported");
        assert_eq!(error.code, "windows_profile_override_unsupported");
    }

    #[test]
    fn public_overrides_never_write_into_private_driver_data() {
        let mut bytes = vec![0xa5; 272];
        bytes[68..70].copy_from_slice(&72_u16.to_le_bytes());
        bytes[70..72].copy_from_slice(&200_u16.to_le_bytes());
        let private = bytes[72..].to_vec();
        let error = apply_public_devmode_overrides(
            &mut bytes,
            &JobOptions {
                copies: Some(2),
                ..Default::default()
            },
        )
        .expect_err("public field is outside dmSize");
        assert_eq!(error.code, "devmode_truncated");
        assert_eq!(&bytes[72..], private);
    }

    #[test]
    fn digest_is_stable_and_prefixed() {
        assert_eq!(
            profile_blob_digest(b"profile fixture"),
            "sha256:f43544dd86059ef7a63432d45bf5f7dd7ebad27abbffa1bddf587a5888ea447e"
        );
    }

    #[test]
    fn page_ranges_are_normalized_and_bounded() {
        assert_eq!(
            selected_pages(Some("3,1-2,2"), 4).expect("range"),
            vec![0, 1, 2]
        );
        assert_eq!(selected_pages(None, 3).expect("all pages"), vec![0, 1, 2]);
        assert_eq!(
            selected_pages(Some("0,2"), 3).expect_err("zero").code,
            "pdf_page_range_invalid"
        );
        assert_eq!(
            selected_pages(Some("2-1"), 3).expect_err("reverse").code,
            "pdf_page_range_invalid"
        );
        assert_eq!(
            selected_pages(Some("4"), 3)
                .expect_err("out of bounds")
                .code,
            "pdf_page_range_invalid"
        );
    }
}
