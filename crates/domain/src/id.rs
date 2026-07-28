use serde::{Deserialize, Serialize};
use std::fmt;
use ulid::Ulid;

macro_rules! typed_id {
    ($name:ident, $prefix:literal) => {
        #[derive(
            Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(Ulid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Ulid::new())
            }

            #[must_use]
            pub const fn from_ulid(value: Ulid) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn as_ulid(self) -> Ulid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}_{}", $prefix, self.0)
            }
        }
    };
}

typed_id!(WorkspaceId, "wsp");
typed_id!(EnvironmentId, "env");
typed_id!(AgentId, "agt");
typed_id!(PrinterId, "ptr");
typed_id!(JobId, "job");
typed_id!(EventId, "evt");
