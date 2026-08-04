//! Owner-only access control for secret files on Windows.
//!
//! Unix restricts the device key with a creation mode. Windows has no
//! equivalent: a newly created file inherits the containing directory's ACL,
//! and a machine-mode installation under `ProgramData` inherits an ACL that
//! grants read access to every authenticated user. A device key protected that
//! way can be copied by any local account and replayed as this node.
//!
//! This module replaces the inherited ACL with a protected, single-entry DACL
//! naming only the file's owner. Inheritance is explicitly severed, so a
//! permissive parent directory cannot re-grant access. Every failure is
//! reported to the caller: a secret that could not be protected must not be
//! treated as written.
//!
//! The Win32 security APIs have no safe binding, so the workspace-wide
//! `unsafe_code` denial is lifted for this module alone. Every block below
//! carries a `SAFETY` note and every call is checked before its result is used.

#![allow(
    unsafe_code,
    reason = "isolated, documented Win32 security calls with no safe equivalent"
)]

use anyhow::{Context, Result, bail};
use std::{os::windows::ffi::OsStrExt as _, path::Path};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, LocalFree},
    Security::{
        ACL, AddAccessAllowedAce,
        Authorization::{SE_FILE_OBJECT, SetNamedSecurityInfoW},
        DACL_SECURITY_INFORMATION, GetLengthSid, InitializeAcl,
        PROTECTED_DACL_SECURITY_INFORMATION, PSID, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    Storage::FileSystem::FILE_ALL_ACCESS,
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

/// Minimum ACL revision that supports the ACE type used here.
const ACL_REVISION: u32 = 2;

/// Restricts `path` so that only its owner may read or write it.
///
/// # Errors
///
/// Returns an error when the process token, the owner SID, the ACL, or the
/// file's security descriptor cannot be read or written.
pub fn restrict_to_owner(path: &Path) -> Result<()> {
    let token = OwnedToken::for_current_process()?;
    let user = token.user_sid()?;
    let mut acl = OwnerOnlyAcl::new(user.sid())?;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();

    // SAFETY: `wide` is NUL-terminated, `acl` points at an initialized ACL that
    // outlives the call, and the null owner/group/SACL arguments are the
    // documented way to leave those parts of the descriptor unchanged.
    let status = unsafe {
        SetNamedSecurityInfoW(
            wide.as_mut_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl.as_mut_ptr(),
            std::ptr::null_mut(),
        )
    };
    if status != 0 {
        bail!(
            "restrict {} to its owner: Windows error {status}",
            path.display()
        );
    }
    Ok(())
}

/// A process token handle that is always closed.
struct OwnedToken(HANDLE);

impl OwnedToken {
    fn for_current_process() -> Result<Self> {
        let mut handle: HANDLE = std::ptr::null_mut();
        // SAFETY: `handle` is a valid out-pointer and the pseudo-handle from
        // `GetCurrentProcess` is always valid for the current process.
        let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut handle) };
        if opened == 0 {
            bail!("open this process's access token");
        }
        Ok(Self(handle))
    }

    fn user_sid(&self) -> Result<TokenUserInformation> {
        let mut needed: u32 = 0;
        // SAFETY: A null buffer with zero length is the documented way to ask
        // for the required size; the call is expected to fail with
        // ERROR_INSUFFICIENT_BUFFER and only writes `needed`.
        unsafe {
            windows_sys::Win32::Security::GetTokenInformation(
                self.0,
                TokenUser,
                std::ptr::null_mut(),
                0,
                &raw mut needed,
            );
        }
        if needed == 0 {
            bail!("size this process's token user information");
        }
        let mut buffer = vec![0_u64; (needed as usize).div_ceil(size_of::<u64>())];
        let capacity = u32::try_from(buffer.len() * size_of::<u64>())
            .context("size this process's token user information")?;
        // SAFETY: `buffer` is at least `needed` bytes and stays alive for the
        // lifetime of the returned value, which borrows the SID inside it.
        let read = unsafe {
            windows_sys::Win32::Security::GetTokenInformation(
                self.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                capacity,
                &raw mut needed,
            )
        };
        if read == 0 {
            bail!("read this process's token user information");
        }
        Ok(TokenUserInformation { buffer })
    }
}

impl Drop for OwnedToken {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from `OpenProcessToken` and is closed once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Owns the buffer that a borrowed owner SID points into.
///
/// Backed by `u64` storage so the buffer satisfies the pointer alignment of the
/// `TOKEN_USER` that the security APIs write into it.
struct TokenUserInformation {
    buffer: Vec<u64>,
}

impl TokenUserInformation {
    const fn sid(&self) -> PSID {
        // SAFETY: `GetTokenInformation` with `TokenUser` fills the buffer with
        // a `TOKEN_USER` whose `User.Sid` points inside that same buffer.
        unsafe { (*self.buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

/// A DACL containing exactly one full-access entry for the owner.
///
/// Backed by `u32` storage so the buffer satisfies the `DWORD` alignment that
/// `InitializeAcl` requires.
struct OwnerOnlyAcl {
    storage: Vec<u32>,
}

impl OwnerOnlyAcl {
    fn new(sid: PSID) -> Result<Self> {
        // SAFETY: `sid` was produced by `GetTokenInformation` and is valid.
        let sid_length = unsafe { GetLengthSid(sid) };
        if sid_length == 0 {
            bail!("measure this process's owner SID");
        }
        // An ACCESS_ALLOWED_ACE ends with the first DWORD of its SID, so the
        // trailing SID bytes beyond that DWORD are the additional space needed.
        let bytes = size_of::<ACL>()
            + size_of::<windows_sys::Win32::Security::ACCESS_ALLOWED_ACE>()
            + sid_length as usize;
        let mut storage = vec![0_u32; bytes.div_ceil(size_of::<u32>())];
        let capacity =
            u32::try_from(storage.len() * size_of::<u32>()).context("size the owner-only ACL")?;

        // SAFETY: `storage` is DWORD-aligned by construction and `capacity`
        // describes its true length in bytes.
        let initialized =
            unsafe { InitializeAcl(storage.as_mut_ptr().cast::<ACL>(), capacity, ACL_REVISION) };
        if initialized == 0 {
            bail!("initialize an owner-only ACL");
        }
        // SAFETY: The ACL was just initialized with room for this single ACE,
        // and `sid` remains valid for the duration of the call.
        let added = unsafe {
            AddAccessAllowedAce(
                storage.as_mut_ptr().cast::<ACL>(),
                ACL_REVISION,
                FILE_ALL_ACCESS,
                sid,
            )
        };
        if added == 0 {
            bail!("grant the owner sole access in a new ACL");
        }
        Ok(Self { storage })
    }

    const fn as_mut_ptr(&mut self) -> *mut ACL {
        self.storage.as_mut_ptr().cast::<ACL>()
    }
}

/// Keeps `LocalFree` linked for the allocator documented by the security APIs.
///
/// The current implementation owns every buffer it passes, so nothing needs
/// freeing; this exists so a future change that adopts an API-allocated
/// descriptor cannot forget the matching deallocation.
#[allow(dead_code, reason = "documents the allocator paired with these APIs")]
const _: unsafe extern "system" fn(*mut core::ffi::c_void) -> *mut core::ffi::c_void = LocalFree;
