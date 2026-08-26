//! Current-user-only Windows named-pipe creation.
//!
//! Win32 has no safe API for attaching a protected DACL to a named pipe. This
//! module is the only unsafe boundary: it obtains the current process token,
//! builds a single-ACE DACL for that user, rejects remote clients, and asks
//! Tokio to create the overlapped pipe with those security attributes.

#![allow(
    unsafe_code,
    reason = "isolated, checked Win32 token and security descriptor calls"
)]

use std::{io, mem::size_of};
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Security::{
        ACCESS_ALLOWED_ACE, ACL, AddAccessAllowedAce, GetLengthSid, GetTokenInformation,
        InitializeAcl, InitializeSecurityDescriptor, PSID, SECURITY_ATTRIBUTES,
        SECURITY_DESCRIPTOR, SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

const ACL_REVISION: u32 = 2;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;
const GENERIC_ALL: u32 = 0x1000_0000;

/// Creates one overlapped broker pipe instance with a current-user-only DACL.
///
/// # Errors
///
/// Returns an error for an invalid endpoint, unavailable process identity,
/// ACL construction failure, or a squatted first pipe instance.
pub fn create_current_user_server(name: &str, first: bool) -> io::Result<NamedPipeServer> {
    validate_pipe_name(name)?;
    let token = OwnedToken::current()?;
    let user = token.user()?;
    let mut acl = OwnerOnlyAcl::new(user.sid())?;
    let mut descriptor = empty_security_descriptor();
    // SAFETY: `descriptor` is writable and has the documented absolute layout.
    if unsafe {
        InitializeSecurityDescriptor((&raw mut descriptor).cast(), SECURITY_DESCRIPTOR_REVISION)
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the initialized descriptor and ACL remain alive until pipe
    // creation returns; the DACL is present and is not inherited/defaulted.
    if unsafe { SetSecurityDescriptorDacl((&raw mut descriptor).cast(), 1, acl.as_mut_ptr(), 0) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
            .map_err(|_| io::Error::other("invalid security attributes size"))?,
        lpSecurityDescriptor: (&raw mut descriptor).cast(),
        bInheritHandle: 0,
    };
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .reject_remote_clients(true);
    // SAFETY: `attributes` points to a live initialized descriptor and DACL;
    // CreateNamedPipe consumes them synchronously and does not retain pointers.
    unsafe { options.create_with_security_attributes_raw(name, (&raw mut attributes).cast()) }
}

fn validate_pipe_name(name: &str) -> io::Result<()> {
    if !name.starts_with(r"\\.\pipe\piqae-node-")
        || name.len() > 240
        || name
            .chars()
            .skip(r"\\.\pipe\".len())
            .any(|character| !character.is_ascii_alphanumeric() && character != '-')
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Piqae broker pipe name",
        ));
    }
    Ok(())
}

struct OwnedToken(HANDLE);

impl OwnedToken {
    fn current() -> io::Result<Self> {
        let mut handle: HANDLE = std::ptr::null_mut();
        // SAFETY: the pseudo process handle is valid and `handle` is writable.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut handle) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(handle))
    }

    fn user(&self) -> io::Result<TokenUserBuffer> {
        let mut needed = 0_u32;
        // SAFETY: null/zero is the documented size query.
        unsafe {
            GetTokenInformation(self.0, TokenUser, std::ptr::null_mut(), 0, &raw mut needed);
        }
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut storage = vec![0_u64; (needed as usize).div_ceil(size_of::<u64>())];
        let capacity = u32::try_from(storage.len() * size_of::<u64>())
            .map_err(|_| io::Error::other("token information is too large"))?;
        // SAFETY: aligned storage has the queried capacity and stays alive.
        if unsafe {
            GetTokenInformation(
                self.0,
                TokenUser,
                storage.as_mut_ptr().cast(),
                capacity,
                &raw mut needed,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(TokenUserBuffer(storage))
    }
}

impl Drop for OwnedToken {
    fn drop(&mut self) {
        // SAFETY: the handle is owned and closed exactly once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct TokenUserBuffer(Vec<u64>);

impl TokenUserBuffer {
    const fn sid(&self) -> PSID {
        // SAFETY: GetTokenInformation wrote TOKEN_USER into this aligned buffer.
        unsafe { (*self.0.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

struct OwnerOnlyAcl(Vec<u32>);

impl OwnerOnlyAcl {
    fn new(sid: PSID) -> io::Result<Self> {
        // SAFETY: the SID is borrowed from a live TOKEN_USER buffer.
        let sid_length = unsafe { GetLengthSid(sid) };
        if sid_length == 0 {
            return Err(io::Error::last_os_error());
        }
        let bytes = size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() + sid_length as usize;
        let mut storage = vec![0_u32; bytes.div_ceil(size_of::<u32>())];
        let capacity = u32::try_from(storage.len() * size_of::<u32>())
            .map_err(|_| io::Error::other("pipe ACL is too large"))?;
        // SAFETY: storage is DWORD-aligned and capacity is exact.
        if unsafe { InitializeAcl(storage.as_mut_ptr().cast(), capacity, ACL_REVISION) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the initialized ACL has room for one ACE and SID remains live.
        if unsafe {
            AddAccessAllowedAce(storage.as_mut_ptr().cast(), ACL_REVISION, GENERIC_ALL, sid)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(storage))
    }

    const fn as_mut_ptr(&mut self) -> *mut ACL {
        self.0.as_mut_ptr().cast()
    }
}

const fn empty_security_descriptor() -> SECURITY_DESCRIPTOR {
    SECURITY_DESCRIPTOR {
        Revision: 0,
        Sbz1: 0,
        Control: 0,
        Owner: std::ptr::null_mut(),
        Group: std::ptr::null_mut(),
        Sacl: std::ptr::null_mut(),
        Dacl: std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_non_piqae_or_remote_pipe_names_before_creation() {
        assert!(validate_pipe_name(r"\\server\pipe\piqae-node-test").is_err());
        assert!(validate_pipe_name(r"\\.\pipe\other").is_err());
        assert!(validate_pipe_name(r"\\.\pipe\piqae-node-valid123").is_ok());
    }

    #[test]
    fn first_instance_collision_fails_closed() {
        let name = format!(r"\\.\pipe\piqae-node-{}", uuid::Uuid::new_v4().simple());
        let first = create_current_user_server(&name, true).expect("first owner-only pipe");
        assert!(create_current_user_server(&name, true).is_err());
        drop(first);
    }
}
