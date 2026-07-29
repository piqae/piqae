//! Windows-native print profile capture and replay foundations.
//!
//! The serializable types and structural validation in this crate are
//! deliberately platform-independent. This lets the opaque native profile
//! envelope be tested without loading a third-party Windows printer driver.

pub mod native_profile;
pub mod replay;

#[cfg(windows)]
pub mod windows_native;

#[cfg(windows)]
pub mod windows_replay;
