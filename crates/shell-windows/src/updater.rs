//! Fail-closed WinSparkle configuration and Windows runtime lifecycle.
//!
//! Update checking is an optional tray-shell concern. The durable agent and
//! local queue continue operating when this configuration is absent or invalid.

use base64::{Engine as _, engine::general_purpose::STANDARD};
#[cfg(windows)]
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, ffi::OsString};
use thiserror::Error;

pub const WINSPARKLE_RUNTIME_FILE: &str = "WinSparkle.dll";
pub const WINSPARKLE_RUNTIME_VERSION: &str = "0.9.4";
#[cfg(windows)]
const MAX_WINSPARKLE_RUNTIME_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdatePolicy {
    Notify,
    Automatic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateConfiguration {
    policy: UpdatePolicy,
    feed_url: String,
    ed25519_public_key: String,
    runtime_sha256: String,
}

impl UpdateConfiguration {
    /// Loads the update trust configuration passed by the signed installer.
    ///
    /// A missing or explicitly disabled policy preserves local printing and
    /// does not load an updater DLL. Any enabled but incomplete configuration
    /// is rejected as a unit.
    ///
    /// # Errors
    ///
    /// Returns an error when an enabled update policy lacks a pinned HTTPS
    /// feed, Ed25519 key, or WinSparkle runtime digest.
    pub fn from_environment() -> Result<Option<Self>, UpdateError> {
        Self::from_values(std::env::vars_os())
    }

    fn from_values(
        environment: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Option<Self>, UpdateError> {
        let environment = environment.into_iter().collect::<BTreeMap<_, _>>();
        let value = |name: &str| {
            environment
                .get(&OsString::from(name))
                .and_then(|value| value.to_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        };
        let policy = match value("PIQAE_UPDATE_POLICY") {
            None | Some("disabled") => return Ok(None),
            Some("notify") => UpdatePolicy::Notify,
            Some("automatic") => UpdatePolicy::Automatic,
            Some(_) => {
                return Err(UpdateError::Configuration(
                    "PIQAE_UPDATE_POLICY must be disabled, notify, or automatic".into(),
                ));
            }
        };

        let feed_url = value("PIQAE_UPDATE_FEED_URL")
            .ok_or_else(|| UpdateError::Configuration("update feed URL is missing".into()))?;
        validate_feed_url(feed_url)?;

        let public_key = value("PIQAE_UPDATE_ED25519_PUBLIC_KEY").ok_or_else(|| {
            UpdateError::Configuration("update verification key is missing".into())
        })?;
        let decoded_key = STANDARD.decode(public_key).map_err(|_| {
            UpdateError::Configuration("update verification key is not valid Base64".into())
        })?;
        if decoded_key.len() != 32 || STANDARD.encode(&decoded_key) != public_key {
            return Err(UpdateError::Configuration(
                "update verification key is not a canonical Ed25519 public key".into(),
            ));
        }

        let runtime_version = value("PIQAE_UPDATE_RUNTIME_VERSION").ok_or_else(|| {
            UpdateError::Configuration("updater runtime version is missing".into())
        })?;
        if runtime_version != WINSPARKLE_RUNTIME_VERSION {
            return Err(UpdateError::Configuration(format!(
                "updater runtime must be WinSparkle {WINSPARKLE_RUNTIME_VERSION}"
            )));
        }
        let runtime_sha256 = value("PIQAE_UPDATE_RUNTIME_SHA256")
            .ok_or_else(|| UpdateError::Configuration("updater runtime digest is missing".into()))?
            .to_ascii_lowercase();
        if runtime_sha256.len() != 64
            || !runtime_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(UpdateError::Configuration(
                "updater runtime digest must be a SHA-256 hex digest".into(),
            ));
        }

        Ok(Some(Self {
            policy,
            feed_url: feed_url.to_owned(),
            ed25519_public_key: public_key.to_owned(),
            runtime_sha256,
        }))
    }

    #[must_use]
    pub const fn policy(&self) -> UpdatePolicy {
        self.policy
    }
}

fn validate_feed_url(value: &str) -> Result<(), UpdateError> {
    let parsed = url::Url::parse(value)
        .map_err(|_| UpdateError::Configuration("update feed URL is invalid".into()))?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(UpdateError::Configuration(
            "update feed URL must be HTTPS without credentials or a fragment".into(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn verify_runtime(
    path: &std::path::Path,
    expected_sha256: &str,
) -> Result<std::fs::File, UpdateError> {
    use std::{io::Read as _, os::windows::fs::OpenOptionsExt as _};
    use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

    let mut options = std::fs::OpenOptions::new();
    options.read(true).share_mode(FILE_SHARE_READ);
    let mut file = options.open(path).map_err(|error| {
        UpdateError::Runtime(format!("cannot open {}: {error}", path.display()))
    })?;
    let metadata = file.metadata().map_err(|error| {
        UpdateError::Runtime(format!("cannot inspect {}: {error}", path.display()))
    })?;
    if metadata.len() == 0 || metadata.len() > MAX_WINSPARKLE_RUNTIME_BYTES {
        return Err(UpdateError::Runtime(
            "WinSparkle runtime has an invalid size".into(),
        ));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            UpdateError::Runtime(format!("cannot read {}: {error}", path.display()))
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual = hex::encode(digest.finalize());
    if actual != expected_sha256 {
        return Err(UpdateError::Runtime(
            "WinSparkle runtime digest does not match signed package metadata".into(),
        ));
    }
    Ok(file)
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("{0}")]
    Configuration(String),
    #[error("{0}")]
    Runtime(String),
}

#[cfg(windows)]
mod windows {
    use super::{
        UpdateConfiguration, UpdateError, UpdatePolicy, WINSPARKLE_RUNTIME_FILE, verify_runtime,
    };
    use std::{
        ffi::{CStr, CString, c_char},
        mem,
        os::windows::ffi::OsStrExt as _,
        path::Path,
    };
    use windows_sys::Win32::{
        Foundation::{FreeLibrary, HMODULE},
        System::LibraryLoader::{
            GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
            LoadLibraryExW,
        },
    };

    type VoidFunction = unsafe extern "C" fn();
    type StringSetter = unsafe extern "C" fn(*const c_char);
    type PublicKeySetter = unsafe extern "C" fn(*const c_char) -> i32;
    type DetailsSetter = unsafe extern "C" fn(*const u16, *const u16, *const u16);
    type IntegerSetter = unsafe extern "C" fn(i32);
    pub type CanShutdownCallback = unsafe extern "C" fn() -> i32;
    pub type ShutdownRequestCallback = unsafe extern "C" fn();
    type CanShutdownSetter = unsafe extern "C" fn(Option<CanShutdownCallback>);
    type ShutdownRequestSetter = unsafe extern "C" fn(Option<ShutdownRequestCallback>);

    /// Owns one initialized WinSparkle runtime on the Windows UI thread.
    pub struct WindowsUpdater {
        module: HMODULE,
        cleanup: VoidFunction,
        check_with_ui: VoidFunction,
        initialized: bool,
    }

    impl WindowsUpdater {
        /// Loads and initializes the pinned runtime beside the tray executable.
        ///
        /// # Errors
        ///
        /// Returns an error before initialization if the runtime, symbols,
        /// feed, or public key cannot be verified.
        pub fn initialize(
            configuration: &UpdateConfiguration,
            executable: &Path,
            can_shutdown: CanShutdownCallback,
            shutdown_request: ShutdownRequestCallback,
        ) -> Result<Self, UpdateError> {
            let directory = executable.parent().ok_or_else(|| {
                UpdateError::Runtime("cannot locate the Windows installation directory".into())
            })?;
            let runtime = directory.join(WINSPARKLE_RUNTIME_FILE);
            // Keep this read handle open with write/delete sharing denied from
            // digest verification through DLL loading. This prevents the path
            // from being replaced between the trust decision and the loader.
            let runtime_guard = verify_runtime(&runtime, &configuration.runtime_sha256)?;
            let wide_runtime = runtime
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();

            // SAFETY: The absolute path is NUL-terminated. Search flags limit
            // dependencies to the loaded DLL directory and System32.
            let module = unsafe {
                LoadLibraryExW(
                    wide_runtime.as_ptr(),
                    std::ptr::null_mut(),
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
                )
            };
            if module.is_null() {
                return Err(UpdateError::Runtime(
                    "cannot load the pinned WinSparkle runtime".into(),
                ));
            }
            drop(runtime_guard);

            let configured =
                unsafe { configure_runtime(module, configuration, can_shutdown, shutdown_request) };
            match configured {
                Ok((cleanup, check_with_ui, initialize)) => {
                    // SAFETY: All configuration setters succeeded and the DLL
                    // remains loaded for the controller lifetime.
                    unsafe { initialize() };
                    Ok(Self {
                        module,
                        cleanup,
                        check_with_ui,
                        initialized: true,
                    })
                }
                Err(error) => {
                    // SAFETY: No WinSparkle initialization occurred and the
                    // handle came from LoadLibraryExW above.
                    unsafe {
                        FreeLibrary(module);
                    }
                    Err(error)
                }
            }
        }

        pub fn check_with_ui(&self) {
            if self.initialized {
                // SAFETY: The function was resolved from the loaded,
                // initialized module and WinSparkle documents it as
                // non-blocking.
                unsafe { (self.check_with_ui)() };
            }
        }
    }

    impl Drop for WindowsUpdater {
        fn drop(&mut self) {
            // SAFETY: Cleanup is called exactly once before unloading the
            // module, as required by the WinSparkle lifecycle.
            unsafe {
                if self.initialized {
                    (self.cleanup)();
                    self.initialized = false;
                }
                FreeLibrary(self.module);
            }
        }
    }

    unsafe fn configure_runtime(
        module: HMODULE,
        configuration: &UpdateConfiguration,
        can_shutdown: CanShutdownCallback,
        shutdown_request: ShutdownRequestCallback,
    ) -> Result<(VoidFunction, VoidFunction, VoidFunction), UpdateError> {
        let set_appcast: StringSetter = unsafe { symbol(module, c"win_sparkle_set_appcast_url")? };
        let set_public_key: PublicKeySetter =
            unsafe { symbol(module, c"win_sparkle_set_eddsa_public_key")? };
        let set_details: DetailsSetter = unsafe { symbol(module, c"win_sparkle_set_app_details")? };
        let set_automatic: IntegerSetter =
            unsafe { symbol(module, c"win_sparkle_set_automatic_check_for_updates")? };
        let set_can_shutdown: CanShutdownSetter =
            unsafe { symbol(module, c"win_sparkle_set_can_shutdown_callback")? };
        let set_shutdown_request: ShutdownRequestSetter =
            unsafe { symbol(module, c"win_sparkle_set_shutdown_request_callback")? };
        let initialize: VoidFunction = unsafe { symbol(module, c"win_sparkle_init")? };
        let cleanup: VoidFunction = unsafe { symbol(module, c"win_sparkle_cleanup")? };
        let check_with_ui: VoidFunction =
            unsafe { symbol(module, c"win_sparkle_check_update_with_ui")? };

        let feed = CString::new(configuration.feed_url.as_str())
            .map_err(|_| UpdateError::Configuration("update feed URL contains NUL".into()))?;
        let public_key = CString::new(configuration.ed25519_public_key.as_str())
            .map_err(|_| UpdateError::Configuration("update public key contains NUL".into()))?;
        let company = wide("Piqae");
        let application = wide("Piqae");
        let version = wide(env!("CARGO_PKG_VERSION"));

        // SAFETY: All function pointers were resolved from this loaded module.
        // Strings live through the calls and are NUL-terminated.
        unsafe {
            set_appcast(feed.as_ptr());
            if set_public_key(public_key.as_ptr()) != 1 {
                return Err(UpdateError::Configuration(
                    "WinSparkle rejected the Ed25519 public key".into(),
                ));
            }
            set_details(company.as_ptr(), application.as_ptr(), version.as_ptr());
            set_automatic(i32::from(configuration.policy() == UpdatePolicy::Automatic));
            set_can_shutdown(Some(can_shutdown));
            set_shutdown_request(Some(shutdown_request));
        }
        Ok((cleanup, check_with_ui, initialize))
    }

    unsafe fn symbol<Function: Copy>(
        module: HMODULE,
        name: &CStr,
    ) -> Result<Function, UpdateError> {
        // SAFETY: The module is live and name is NUL-terminated.
        let address = unsafe { GetProcAddress(module, name.as_ptr().cast()) }.ok_or_else(|| {
            UpdateError::Runtime(format!(
                "WinSparkle runtime is missing {}",
                name.to_string_lossy()
            ))
        })?;
        if mem::size_of::<Function>() != mem::size_of_val(&address) {
            return Err(UpdateError::Runtime(
                "WinSparkle function pointer has an unsupported size".into(),
            ));
        }
        // SAFETY: The symbol names above are bound to the exact WinSparkle C
        // declarations and the pointer size was checked.
        Ok(unsafe { mem::transmute_copy(&address) })
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub use WindowsUpdater as Controller;
}

#[cfg(windows)]
pub use windows::{CanShutdownCallback, Controller as WindowsUpdater, ShutdownRequestCallback};

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_environment(policy: &str) -> Vec<(OsString, OsString)> {
        vec![
            ("PIQAE_UPDATE_POLICY", policy),
            (
                "PIQAE_UPDATE_FEED_URL",
                "https://downloads.piqae.com/releases/stable/appcast-windows.xml",
            ),
            (
                "PIQAE_UPDATE_ED25519_PUBLIC_KEY",
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            ),
            ("PIQAE_UPDATE_RUNTIME_VERSION", WINSPARKLE_RUNTIME_VERSION),
            (
                "PIQAE_UPDATE_RUNTIME_SHA256",
                "9b43b1c16ee39fb9a91b5bd75138767898779510e0836be2919250607cdbe8ab",
            ),
        ]
        .into_iter()
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
    }

    #[test]
    fn disabled_or_absent_policy_does_not_require_updater_files() {
        assert_eq!(
            UpdateConfiguration::from_values(std::iter::empty()).unwrap(),
            None
        );
        assert_eq!(
            UpdateConfiguration::from_values([(
                OsString::from("PIQAE_UPDATE_POLICY"),
                OsString::from("disabled"),
            )])
            .unwrap(),
            None
        );
    }

    #[test]
    fn enabled_policy_requires_a_complete_trust_tuple() {
        for omitted in [
            "PIQAE_UPDATE_FEED_URL",
            "PIQAE_UPDATE_ED25519_PUBLIC_KEY",
            "PIQAE_UPDATE_RUNTIME_VERSION",
            "PIQAE_UPDATE_RUNTIME_SHA256",
        ] {
            let environment = enabled_environment("notify")
                .into_iter()
                .filter(|(name, _)| name != omitted)
                .collect::<Vec<_>>();
            assert!(
                UpdateConfiguration::from_values(environment).is_err(),
                "{omitted} should be required"
            );
        }
    }

    #[test]
    fn feed_and_public_key_are_validated_before_runtime_loading() {
        for feed in [
            "http://downloads.piqae.com/appcast.xml",
            "https://user:pass@downloads.piqae.com/appcast.xml",
            "https://downloads.piqae.com/appcast.xml#old",
            "not a URL",
        ] {
            let mut environment = enabled_environment("notify");
            let value = environment
                .iter_mut()
                .find(|(name, _)| name == "PIQAE_UPDATE_FEED_URL")
                .unwrap_or_else(|| panic!("missing test feed"));
            value.1 = OsString::from(feed);
            assert!(UpdateConfiguration::from_values(environment).is_err());
        }

        let mut environment = enabled_environment("notify");
        let value = environment
            .iter_mut()
            .find(|(name, _)| name == "PIQAE_UPDATE_ED25519_PUBLIC_KEY")
            .unwrap_or_else(|| panic!("missing test key"));
        value.1 = OsString::from("not-a-public-key");
        assert!(UpdateConfiguration::from_values(environment).is_err());
    }

    #[test]
    fn policy_distinguishes_manual_notification_from_automatic_checks() {
        let notify = UpdateConfiguration::from_values(enabled_environment("notify"))
            .unwrap()
            .unwrap_or_else(|| panic!("notify should enable updates"));
        let automatic = UpdateConfiguration::from_values(enabled_environment("automatic"))
            .unwrap()
            .unwrap_or_else(|| panic!("automatic should enable updates"));
        assert_eq!(notify.policy(), UpdatePolicy::Notify);
        assert_eq!(automatic.policy(), UpdatePolicy::Automatic);
    }
}
