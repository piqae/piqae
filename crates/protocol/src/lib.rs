//! Wire contracts. Add fields compatibly and support protocol N and N-1.

pub mod agent;
pub mod executor;

pub const CURRENT_PROTOCOL_VERSION: u16 = 1;
pub const MINIMUM_PROTOCOL_VERSION: u16 = 1;
