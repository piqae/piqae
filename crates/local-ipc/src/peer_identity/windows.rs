//! Windows named-pipe peer verification.

#![allow(
    unsafe_code,
    reason = "isolated checked Win32 process, token, package and WinVerifyTrust boundary"
)]

use super::PeerApplicationEvidence;
use crate::BrokerApplicationIdentity;
use sha2::{Digest as _, Sha256};
use std::{
    ffi::OsString,
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
    mem::{MaybeUninit, size_of},
    os::windows::{
        ffi::{OsStrExt as _, OsStringExt as _},
        io::AsRawHandle as _,
    },
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, FILETIME, HANDLE},
    Security::{
        EqualSid, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser,
        WinTrust::{
            WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0, WINTRUST_FILE_INFO,
            WTD_CACHE_ONLY_URL_RETRIEVAL, WTD_CHOICE_FILE, WTD_REVOCATION_CHECK_NONE,
            WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE, WTD_STATEACTION_VERIFY, WTD_UI_NONE,
            WTHelperGetProvCertFromChain, WTHelperGetProvSignerFromChain,
            WTHelperProvDataFromStateData, WinVerifyTrust,
        },
    },
    Storage::Packaging::Appx::GetPackageFamilyName,
    System::{
        Pipes::GetNamedPipeClientProcessId,
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            GetCurrentProcess, GetCurrentProcessId, GetProcessTimes, OpenProcess, OpenProcessToken,
            PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
    },
};

const APPMODEL_ERROR_NO_PACKAGE: u32 = 15_700;
const MAX_IMAGE_PATH_CHARS: usize = 32_768;
const MAX_SIGNED_IMAGE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct OwnedProcess(HANDLE);

impl Drop for OwnedProcess {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the handle came from OpenProcess and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

impl OwnedProcess {
    #[cfg(any(test, feature = "test-peer-identity"))]
    pub(super) const fn test_sentinel() -> Self {
        Self(std::ptr::null_mut())
    }
}

pub(super) fn verify(
    pipe: &tokio::net::windows::named_pipe::NamedPipeServer,
) -> std::io::Result<(PeerApplicationEvidence, OwnedProcess)> {
    let mut process_id = 0_u32;
    // SAFETY: the handle belongs to the connected pipe and process_id is writable.
    if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle().cast(), &raw mut process_id) } == 0
        || process_id == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: access is query-only and the PID came from the connected pipe.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let process = OwnedProcess(process);
    verify_same_user_and_session(process_id, process.0)?;

    let image_path = process_image_path(process.0)?;
    let canonical_path = std::fs::canonicalize(&image_path)
        .map_err(|error| std::io::Error::new(error.kind(), "peer executable is unavailable"))?;
    let canonical = normalize_windows_path(&canonical_path)?;
    let mut image = File::open(&canonical_path)
        .map_err(|error| std::io::Error::new(error.kind(), "peer executable is unavailable"))?;
    let image_hash = bounded_file_sha256(&mut image)?;
    let (application_id, principal_material, signing_identity_sha256) =
        if let Some(family) = package_family(process.0)? {
            let family = family.to_ascii_lowercase();
            let family_material = format!("package-family\0{family}");
            (
                family,
                family_material.clone(),
                hex::encode(Sha256::digest(family_material.as_bytes())),
            )
        } else {
            let signer_sha256 = verify_authenticode(&canonical_path, &image)?;
            let principal = format!("authenticode-app-v1\0{canonical}\0{signer_sha256}");
            let derived = hex::encode(Sha256::digest(principal.as_bytes()));
            (
                format!("win32.{}", &derived[..32]),
                principal,
                signer_sha256,
            )
        };
    if application_id.is_empty()
        || application_id.len() > 255
        || !application_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(denied(
            "the broker peer's verified application id is unsupported",
        ));
    }
    let principal_sha256 = hex::encode(Sha256::digest(
        [
            b"piqae-peer-principal-v1\0windows\0".as_slice(),
            principal_material.as_bytes(),
        ]
        .concat(),
    ));
    let creation_time = process_creation_time(process.0)?;
    let process_instance_sha256 = hex::encode(Sha256::digest(format!(
        "windows-process-v1\0{process_id}\0{creation_time}\0{canonical}\0{image_hash}"
    )));
    let display_name = canonical_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .unwrap_or(&application_id)
        .to_owned();
    Ok((
        PeerApplicationEvidence::verified(
            BrokerApplicationIdentity {
                application_id,
                display_name,
                signing_identity_sha256: Some(signing_identity_sha256),
            },
            principal_sha256,
            process_instance_sha256,
            "windows",
            process_id,
        ),
        process,
    ))
}

fn verify_same_user_and_session(process_id: u32, process: HANDLE) -> std::io::Result<()> {
    let client = ProcessToken::open(process)?;
    // SAFETY: GetCurrentProcess returns a valid pseudo-handle.
    let server = ProcessToken::open(unsafe { GetCurrentProcess() })?;
    let client_user = client.user()?;
    let server_user = server.user()?;
    // SAFETY: both pointers refer to TOKEN_USER buffers kept alive here.
    if unsafe { EqualSid(client_user.sid(), server_user.sid()) } == 0 {
        return Err(denied("broker peer belongs to another Windows user"));
    }
    let mut client_session = 0_u32;
    let mut server_session = 0_u32;
    // SAFETY: both session pointers are writable and PIDs are valid snapshots.
    if unsafe { ProcessIdToSessionId(process_id, &raw mut client_session) } == 0
        || unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &raw mut server_session) } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    if client_session != server_session {
        return Err(denied("broker peer belongs to another Windows session"));
    }
    Ok(())
}

struct ProcessToken(HANDLE);

impl ProcessToken {
    fn open(process: HANDLE) -> std::io::Result<Self> {
        let mut token = ptr::null_mut();
        // SAFETY: process is a live process handle and token is writable.
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut token) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self(token))
    }

    fn user(&self) -> std::io::Result<TokenUserBuffer> {
        let mut needed = 0_u32;
        // SAFETY: this is the documented size query; failure is expected.
        unsafe {
            GetTokenInformation(self.0, TokenUser, ptr::null_mut(), 0, &raw mut needed);
        }
        if needed == 0 || needed > 64 * 1024 {
            return Err(std::io::Error::last_os_error());
        }
        let mut words = vec![
            0_usize;
            usize::try_from(needed)
                .unwrap_or(0)
                .div_ceil(size_of::<usize>())
        ];
        // SAFETY: the aligned word buffer is writable for at least needed bytes.
        if unsafe {
            GetTokenInformation(
                self.0,
                TokenUser,
                words.as_mut_ptr().cast(),
                needed,
                &raw mut needed,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(TokenUserBuffer(words))
    }
}

impl Drop for ProcessToken {
    fn drop(&mut self) {
        // SAFETY: handle came from OpenProcessToken and is closed once.
        unsafe { CloseHandle(self.0) };
    }
}

struct TokenUserBuffer(Vec<usize>);

impl TokenUserBuffer {
    fn sid(&self) -> windows_sys::Win32::Security::PSID {
        // SAFETY: GetTokenInformation initialized TOKEN_USER at this aligned address.
        unsafe { self.0.as_ptr().cast::<TOKEN_USER>().as_ref() }
            .map_or(ptr::null_mut(), |user| user.User.Sid)
    }
}

fn process_image_path(process: HANDLE) -> std::io::Result<PathBuf> {
    let mut path = vec![0_u16; MAX_IMAGE_PATH_CHARS];
    let mut length = u32::try_from(path.len()).map_err(|_| invalid("image path too long"))?;
    // SAFETY: process is held live and path points to `length` writable UTF-16 units.
    if unsafe { QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &raw mut length) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    path.truncate(usize::try_from(length).map_err(|_| invalid("image path too long"))?);
    if path.is_empty() || path.contains(&0) {
        return Err(invalid("peer executable path is malformed"));
    }
    Ok(PathBuf::from(OsString::from_wide(&path)))
}

fn process_creation_time(process: HANDLE) -> std::io::Result<u64> {
    let empty = || FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut creation = empty();
    let mut exit = empty();
    let mut kernel = empty();
    let mut user = empty();
    // SAFETY: process is held live and all FILETIME outputs are writable.
    if unsafe {
        GetProcessTimes(
            process,
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

fn package_family(process: HANDLE) -> std::io::Result<Option<String>> {
    let mut length = 0_u32;
    // SAFETY: documented size query for held process handle.
    let result = unsafe { GetPackageFamilyName(process, &raw mut length, ptr::null_mut()) };
    if result == APPMODEL_ERROR_NO_PACKAGE {
        return Ok(None);
    }
    if result != ERROR_INSUFFICIENT_BUFFER || !(2..=512).contains(&length) {
        return Err(std::io::Error::from_raw_os_error(
            i32::try_from(result).unwrap_or(i32::MAX),
        ));
    }
    let mut family =
        vec![0_u16; usize::try_from(length).map_err(|_| invalid("package id too long"))?];
    // SAFETY: family has the size requested by the first call.
    let result = unsafe { GetPackageFamilyName(process, &raw mut length, family.as_mut_ptr()) };
    if result != 0 {
        return Err(std::io::Error::from_raw_os_error(
            i32::try_from(result).unwrap_or(i32::MAX),
        ));
    }
    let used = family
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(family.len());
    String::from_utf16(&family[..used])
        .map(Some)
        .map_err(|_| invalid("package family is invalid UTF-16"))
}

fn verify_authenticode(path: &Path, mut image: &File) -> std::io::Result<String> {
    image.seek(SeekFrom::Start(0))?;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: u32::try_from(size_of::<WINTRUST_FILE_INFO>())
            .map_err(|_| invalid("WinTrust structure size overflow"))?,
        pcwszFilePath: wide.as_ptr(),
        hFile: image.as_raw_handle().cast(),
        pgKnownSubject: ptr::null_mut(),
    };
    let mut data: WINTRUST_DATA = unsafe { MaybeUninit::zeroed().assume_init() };
    data.cbStruct = u32::try_from(size_of::<WINTRUST_DATA>())
        .map_err(|_| invalid("WinTrust structure size overflow"))?;
    data.dwUIChoice = WTD_UI_NONE;
    data.fdwRevocationChecks = WTD_REVOKE_NONE;
    data.dwUnionChoice = WTD_CHOICE_FILE;
    data.Anonymous = WINTRUST_DATA_0 {
        pFile: &raw mut file,
    };
    data.dwStateAction = WTD_STATEACTION_VERIFY;
    data.dwProvFlags = WTD_CACHE_ONLY_URL_RETRIEVAL | WTD_REVOCATION_CHECK_NONE;
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    // SAFETY: WinTrust structures and path remain alive for this synchronous call.
    let status = unsafe {
        WinVerifyTrust(
            ptr::null_mut(),
            &raw mut action,
            std::ptr::from_mut(&mut data).cast(),
        )
    };
    if status != 0 {
        close_wintrust_state(&mut action, &mut data);
        return Err(denied(
            "the broker peer does not have a valid Authenticode signature",
        ));
    }
    let signer = signer_certificate_sha256(data.hWVTStateData);
    close_wintrust_state(&mut action, &mut data);
    signer
}

fn signer_certificate_sha256(state: HANDLE) -> std::io::Result<String> {
    if state.is_null() {
        return Err(denied("WinTrust returned no verified signer state"));
    }
    // SAFETY: state was returned by a successful WinVerifyTrust VERIFY action
    // and remains valid until the matching CLOSE action below.
    let provider = unsafe { WTHelperProvDataFromStateData(state) };
    if provider.is_null() {
        return Err(denied("WinTrust returned no verified provider"));
    }
    // SAFETY: provider is the checked WinTrust state for the primary signer.
    let signer = unsafe { WTHelperGetProvSignerFromChain(provider, 0, 0, 0) };
    if signer.is_null() {
        return Err(denied("WinTrust returned no primary signer"));
    }
    // SAFETY: signer is the checked primary signer and index zero is its leaf
    // signing certificate.
    let provider_certificate = unsafe { WTHelperGetProvCertFromChain(signer, 0) };
    if provider_certificate.is_null() {
        return Err(denied("WinTrust returned no signer certificate"));
    }
    // SAFETY: provider certificate remains owned by WinTrust state here.
    let certificate = unsafe { (*provider_certificate).pCert };
    if certificate.is_null() {
        return Err(denied("WinTrust signer certificate is unavailable"));
    }
    // SAFETY: checked certificate context belongs to live WinTrust state.
    let length = usize::try_from(unsafe { (*certificate).cbCertEncoded })
        .map_err(|_| invalid("WinTrust signer certificate is too large"))?;
    if !(1..=1024 * 1024).contains(&length) {
        return Err(denied("WinTrust signer certificate is malformed"));
    }
    // SAFETY: certificate reports `length` encoded bytes for its live context.
    let encoded = unsafe { (*certificate).pbCertEncoded };
    if encoded.is_null() {
        return Err(denied("WinTrust signer certificate is unreadable"));
    }
    // SAFETY: pointer and bounded length came from the live certificate.
    Ok(hex::encode(Sha256::digest(unsafe {
        std::slice::from_raw_parts(encoded, length)
    })))
}

fn close_wintrust_state(action: &mut windows_sys::core::GUID, data: &mut WINTRUST_DATA) {
    if data.hWVTStateData.is_null() {
        return;
    }
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    // SAFETY: this closes exactly the state created by the preceding VERIFY.
    let _ = unsafe { WinVerifyTrust(ptr::null_mut(), action, std::ptr::from_mut(data).cast()) };
    data.hWVTStateData = ptr::null_mut();
}

fn bounded_file_sha256(file: &mut File) -> std::io::Result<String> {
    let size = file.seek(SeekFrom::End(0))?;
    if size == 0 || size > MAX_SIGNED_IMAGE_BYTES {
        return Err(invalid("peer executable is outside supported bounds"));
    }
    file.seek(SeekFrom::Start(0))?;
    let mut hash = Sha256::new();
    let mut remaining = size;
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    while remaining > 0 {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Err(invalid("peer executable changed while being verified"));
        }
        remaining = remaining.saturating_sub(u64::try_from(count).unwrap_or(u64::MAX));
        hash.update(&buffer[..count]);
    }
    Ok(hex::encode(hash.finalize()))
}

fn normalize_windows_path(path: &Path) -> std::io::Result<String> {
    let value = path
        .to_str()
        .ok_or_else(|| invalid("peer executable path is not Unicode"))?
        .replace('/', "\\")
        .to_lowercase();
    if value.is_empty() || value.len() > MAX_IMAGE_PATH_CHARS {
        return Err(invalid("peer executable path is outside supported bounds"));
    }
    Ok(value)
}

fn invalid(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

fn denied(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, message)
}
