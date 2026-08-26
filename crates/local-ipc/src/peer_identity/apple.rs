//! macOS peer verification using the accepted socket's audit token and
//! Security.framework dynamic-code APIs.

#![allow(
    unsafe_code,
    reason = "isolated Darwin socket, BSM audit-token and Security.framework boundary"
)]

use super::PeerApplicationEvidence;
use crate::BrokerApplicationIdentity;
use sha2::{Digest as _, Sha256};
use std::{
    ffi::{CStr, c_char, c_int, c_uint, c_void},
    mem::{MaybeUninit, size_of},
    os::fd::AsRawFd as _,
    ptr,
};

type CFIndex = isize;
type CFTypeId = usize;
type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFDataRef = *const c_void;
type CFDictionaryRef = *const c_void;
type SecCodeRef = *mut c_void;
type SecRequirementRef = *mut c_void;

const SOL_LOCAL: c_int = 0;
const LOCAL_PEERTOKEN: c_int = 0x006;
const UTF8: u32 = 0x0800_0100;
const SEC_CS_STRICT_VALIDATE: u32 = 1 << 4;
const SEC_CS_NO_NETWORK_ACCESS: u32 = 1 << 29;
const SEC_CS_SIGNING_INFORMATION: u32 = 1 << 1;
const SEC_CS_REQUIREMENT_INFORMATION: u32 = 1 << 2;

#[repr(C)]
#[derive(Clone, Copy)]
struct AuditToken {
    value: [c_uint; 8],
}

#[link(name = "bsm")]
unsafe extern "C" {
    fn audit_token_to_euid(token: AuditToken) -> u32;
    fn audit_token_to_pid(token: AuditToken) -> c_int;
    fn audit_token_to_pidversion(token: AuditToken) -> c_int;
}

#[link(name = "System")]
unsafe extern "C" {
    fn geteuid() -> u32;
    fn getsockopt(
        socket: c_int,
        level: c_int,
        name: c_int,
        value: *mut c_void,
        length: *mut u32,
    ) -> c_int;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFAllocatorDefault: *const c_void;
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    fn CFDataCreate(allocator: *const c_void, bytes: *const u8, length: CFIndex) -> CFDataRef;
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: CFIndex,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFDictionaryRef;
    fn CFDictionaryGetValue(dictionary: CFDictionaryRef, key: *const c_void) -> *const c_void;
    fn CFGetTypeID(value: CFTypeRef) -> CFTypeId;
    fn CFStringGetTypeID() -> CFTypeId;
    fn CFDataGetTypeID() -> CFTypeId;
    fn CFStringGetCStringPtr(value: CFStringRef, encoding: u32) -> *const c_char;
    fn CFStringGetLength(value: CFStringRef) -> CFIndex;
    fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
    fn CFStringGetCString(
        value: CFStringRef,
        buffer: *mut c_char,
        buffer_size: CFIndex,
        encoding: u32,
    ) -> u8;
    fn CFDataGetLength(value: CFDataRef) -> CFIndex;
    fn CFDataGetBytePtr(value: CFDataRef) -> *const u8;
    fn CFRelease(value: CFTypeRef);
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecGuestAttributeAudit: CFStringRef;
    static kSecCodeInfoIdentifier: CFStringRef;
    static kSecCodeInfoTeamIdentifier: CFStringRef;
    static kSecCodeInfoDesignatedRequirement: CFStringRef;
    static kSecCodeInfoUnique: CFStringRef;
    fn SecCodeCopyGuestWithAttributes(
        host: SecCodeRef,
        attributes: CFDictionaryRef,
        flags: u32,
        guest: *mut SecCodeRef,
    ) -> i32;
    fn SecCodeCheckValidity(code: SecCodeRef, flags: u32, requirement: SecRequirementRef) -> i32;
    fn SecCodeCopySigningInformation(
        code: SecCodeRef,
        flags: u32,
        information: *mut CFDictionaryRef,
    ) -> i32;
    fn SecRequirementCopyString(
        requirement: SecRequirementRef,
        flags: u32,
        text: *mut CFStringRef,
    ) -> i32;
}

pub(super) fn verify(stream: &tokio::net::UnixStream) -> std::io::Result<PeerApplicationEvidence> {
    let credentials = stream.peer_cred()?;
    let pid = credentials
        .pid()
        .ok_or_else(|| invalid("peer PID unavailable"))?;
    let pid = u32::try_from(pid).map_err(|_| invalid("peer PID is invalid"))?;
    // SAFETY: geteuid has no preconditions.
    if credentials.uid() != unsafe { geteuid() } {
        return Err(denied("broker peer belongs to another user"));
    }

    let token = socket_audit_token(stream)?;
    // SAFETY: token was returned at the documented LOCAL_PEERTOKEN size.
    let audit_process_id = unsafe { audit_token_to_pid(token) };
    // SAFETY: same as above.
    let audit_effective_user = unsafe { audit_token_to_euid(token) };
    if audit_process_id != i32::try_from(pid).map_err(|_| invalid("peer PID is invalid"))?
        || audit_effective_user != credentials.uid()
    {
        return Err(denied("socket peer credentials and audit token disagree"));
    }

    let audit_bytes = token_bytes(&token);
    let signing = code_signing_evidence(&audit_bytes)?;
    let normalized_team_id = signing.team_id.map(|team| team.to_ascii_uppercase());
    let signing_material = format!(
        "apple-v1\0{}\0{}\0{}",
        normalized_team_id.as_deref().unwrap_or("adhoc"),
        signing.identifier,
        signing.designated_requirement
    );
    let signing_identity_sha256 = hex::encode(Sha256::digest(signing_material.as_bytes()));
    let principal_sha256 = hex::encode(Sha256::digest(
        [
            b"piqae-peer-principal-v1\0".as_slice(),
            signing_material.as_bytes(),
        ]
        .concat(),
    ));
    // Include the kernel PID generation and code-directory hash so a reused PID
    // cannot inherit evidence captured for an earlier process.
    // SAFETY: token is kernel-provided and valid for the BSM accessor.
    let pid_version = unsafe { audit_token_to_pidversion(token) };
    let process_material = format!(
        "apple-process-v1\0{pid}\0{pid_version}\0{}",
        signing.code_directory_hash
    );
    let process_instance_sha256 = hex::encode(Sha256::digest(process_material.as_bytes()));
    let display_name = signing.identifier.clone();
    Ok(PeerApplicationEvidence::verified(
        BrokerApplicationIdentity {
            application_id: signing.identifier,
            display_name,
            signing_identity_sha256: Some(signing_identity_sha256),
        },
        principal_sha256,
        process_instance_sha256,
        "macos",
        pid,
    ))
}

fn socket_audit_token(stream: &tokio::net::UnixStream) -> std::io::Result<AuditToken> {
    let mut token = MaybeUninit::<AuditToken>::uninit();
    let mut length = u32::try_from(size_of::<AuditToken>())
        .map_err(|_| invalid("audit token size is invalid"))?;
    // SAFETY: token points to writable storage of `length` bytes and the file
    // descriptor belongs to the live accepted Unix stream.
    let result = unsafe {
        getsockopt(
            stream.as_raw_fd(),
            SOL_LOCAL,
            LOCAL_PEERTOKEN,
            token.as_mut_ptr().cast(),
            &raw mut length,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    if usize::try_from(length).ok() != Some(size_of::<AuditToken>()) {
        return Err(invalid("kernel returned a malformed audit token"));
    }
    // SAFETY: getsockopt succeeded and returned the exact structure size.
    Ok(unsafe { token.assume_init() })
}

#[allow(
    clippy::missing_const_for_fn,
    reason = "copy_nonoverlapping is kept in the documented unsafe platform boundary"
)]
fn token_bytes(token: &AuditToken) -> [u8; size_of::<AuditToken>()] {
    let mut bytes = [0_u8; size_of::<AuditToken>()];
    // SAFETY: both buffers are exactly the same size and do not overlap.
    unsafe {
        ptr::copy_nonoverlapping(
            std::ptr::from_ref(token).cast::<u8>(),
            bytes.as_mut_ptr(),
            bytes.len(),
        );
    }
    bytes
}

struct SigningEvidence {
    identifier: String,
    team_id: Option<String>,
    designated_requirement: String,
    code_directory_hash: String,
}

fn code_signing_evidence(audit_token: &[u8]) -> std::io::Result<SigningEvidence> {
    // SAFETY: all CF/Security values are checked for null/type/status and every
    // create-rule value is released exactly once within this function.
    unsafe {
        let audit_data = CFDataCreate(
            kCFAllocatorDefault,
            audit_token.as_ptr(),
            isize::try_from(audit_token.len()).map_err(|_| invalid("audit token too large"))?,
        );
        if audit_data.is_null() {
            return Err(invalid("could not create audit-token evidence"));
        }
        let keys = [kSecGuestAttributeAudit.cast::<c_void>()];
        let values = [audit_data.cast::<c_void>()];
        let attributes = CFDictionaryCreate(
            kCFAllocatorDefault,
            keys.as_ptr(),
            values.as_ptr(),
            1,
            std::ptr::addr_of!(kCFTypeDictionaryKeyCallBacks),
            std::ptr::addr_of!(kCFTypeDictionaryValueCallBacks),
        );
        CFRelease(audit_data);
        if attributes.is_null() {
            return Err(invalid("could not create code-signing attributes"));
        }
        let mut code = ptr::null_mut();
        let copy_status =
            SecCodeCopyGuestWithAttributes(ptr::null_mut(), attributes, 0, &raw mut code);
        CFRelease(attributes);
        if copy_status != 0 || code.is_null() {
            return Err(denied(
                "Security.framework could not identify the broker peer",
            ));
        }
        let validity = SecCodeCheckValidity(
            code,
            SEC_CS_STRICT_VALIDATE | SEC_CS_NO_NETWORK_ACCESS,
            ptr::null_mut(),
        );
        if validity != 0 {
            CFRelease(code);
            return Err(denied("the broker peer's code signature is invalid"));
        }
        let mut information = ptr::null();
        let signing_status = SecCodeCopySigningInformation(
            code,
            SEC_CS_SIGNING_INFORMATION | SEC_CS_REQUIREMENT_INFORMATION,
            &raw mut information,
        );
        CFRelease(code);
        if signing_status != 0 || information.is_null() {
            return Err(denied(
                "the broker peer has no verifiable signing information",
            ));
        }
        let result = extract_signing_information(information);
        CFRelease(information);
        result
    }
}

unsafe fn extract_signing_information(
    information: CFDictionaryRef,
) -> std::io::Result<SigningEvidence> {
    let identifier = unsafe { dictionary_string(information, kSecCodeInfoIdentifier) }?
        .ok_or_else(|| denied("the broker peer is unsigned"))?;
    let team_id = unsafe { dictionary_string(information, kSecCodeInfoTeamIdentifier) }?;
    let requirement = unsafe {
        CFDictionaryGetValue(
            information,
            kSecCodeInfoDesignatedRequirement.cast::<c_void>(),
        )
    };
    if requirement.is_null() {
        return Err(denied("the broker peer has no designated requirement"));
    }
    let mut requirement_text = ptr::null();
    // SAFETY: requirement came from the checked Security.framework dictionary.
    if unsafe { SecRequirementCopyString(requirement.cast_mut(), 0, &raw mut requirement_text) }
        != 0
        || requirement_text.is_null()
    {
        return Err(denied(
            "the broker peer's designated requirement is unreadable",
        ));
    }
    let designated_requirement = unsafe { cf_string(requirement_text) };
    // SAFETY: create-rule string from SecRequirementCopyString.
    unsafe { CFRelease(requirement_text) };
    let designated_requirement = designated_requirement?;

    let unique = unsafe { CFDictionaryGetValue(information, kSecCodeInfoUnique.cast::<c_void>()) };
    if unique.is_null() || unsafe { CFGetTypeID(unique) } != unsafe { CFDataGetTypeID() } {
        return Err(denied("the broker peer has no code-directory hash"));
    }
    let length = unsafe { CFDataGetLength(unique) };
    if !(16..=64).contains(&length) {
        return Err(denied("the broker peer's code-directory hash is malformed"));
    }
    let bytes = unsafe { CFDataGetBytePtr(unique) };
    if bytes.is_null() {
        return Err(denied(
            "the broker peer's code-directory hash is unreadable",
        ));
    }
    let code_directory_hash = hex::encode(unsafe {
        std::slice::from_raw_parts(
            bytes,
            usize::try_from(length).map_err(|_| invalid("code-directory hash too large"))?,
        )
    });
    if identifier.len() > 255
        || identifier.is_empty()
        || !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        || team_id.as_ref().is_some_and(|team| {
            team.is_empty()
                || team.len() > 128
                || !team.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        || designated_requirement.is_empty()
        || designated_requirement.len() > 8_192
    {
        return Err(denied(
            "the broker peer's signing identity is outside supported bounds",
        ));
    }
    Ok(SigningEvidence {
        identifier,
        team_id,
        designated_requirement,
        code_directory_hash,
    })
}

unsafe fn dictionary_string(
    dictionary: CFDictionaryRef,
    key: CFStringRef,
) -> std::io::Result<Option<String>> {
    let value = unsafe { CFDictionaryGetValue(dictionary, key.cast::<c_void>()) };
    if value.is_null() {
        return Ok(None);
    }
    if unsafe { CFGetTypeID(value) } != unsafe { CFStringGetTypeID() } {
        return Err(denied(
            "Security.framework returned malformed signing information",
        ));
    }
    unsafe { cf_string(value) }.map(Some)
}

unsafe fn cf_string(value: CFStringRef) -> std::io::Result<String> {
    let direct = unsafe { CFStringGetCStringPtr(value, UTF8) };
    if !direct.is_null() {
        return unsafe { CStr::from_ptr(direct) }
            .to_str()
            .map(str::to_owned)
            .map_err(|_| invalid("Security.framework returned invalid UTF-8"));
    }
    let length = unsafe { CFStringGetLength(value) };
    let maximum = unsafe { CFStringGetMaximumSizeForEncoding(length, UTF8) };
    if !(0..=32_768).contains(&maximum) {
        return Err(invalid(
            "Security.framework string exceeds supported bounds",
        ));
    }
    let capacity = usize::try_from(maximum)
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| invalid("Security.framework string size overflow"))?;
    let mut bytes = vec![0_u8; capacity];
    let capacity = isize::try_from(capacity)
        .map_err(|_| invalid("Security.framework string size overflow"))?;
    if unsafe { CFStringGetCString(value, bytes.as_mut_ptr().cast(), capacity, UTF8) } == 0 {
        return Err(invalid("Security.framework string conversion failed"));
    }
    CStr::from_bytes_until_nul(&bytes)
        .map_err(|_| invalid("Security.framework returned an unterminated string"))?
        .to_str()
        .map(str::to_owned)
        .map_err(|_| invalid("Security.framework returned invalid UTF-8"))
}

fn invalid(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, message)
}

fn denied(message: &'static str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, message)
}
