use crate::native_profile::{
    NativeProfileError, WindowsDriverFingerprint, WindowsNativeProfileCapture,
};
use serde::{Deserialize, Serialize};
use spool_domain::JobOptions;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowsPdfBackend {
    /// The intended certified backend. Rendering is not implemented yet.
    GdiPdfium,
    /// Generic preview compatibility path. It cannot replay native profiles.
    SumatraPreview,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProfileReplayPlan {
    /// The validated, captured DEVMODE. It is not yet normalized with the
    /// requested overrides; the future Windows backend must apply supported
    /// fields and call DocumentPropertiesW before creating a device context.
    pub devmode_bytes: Vec<u8>,
    pub requested_overrides: BTreeSet<String>,
}

/// Validates everything that can be checked before native PDF rendering.
///
/// Building this plan does not apply overrides, render PDF pages, create a
/// device context, or submit a job. The executor must not report native-profile
/// support until those later stages are implemented.
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
}
