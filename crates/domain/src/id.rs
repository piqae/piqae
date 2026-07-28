use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use ulid::Ulid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseTypedIdError {
    value: String,
    expected_prefix: &'static str,
}

impl fmt::Display for ParseTypedIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid identifier {:?}; expected {}_<ULID> or a bare ULID",
            self.value, self.expected_prefix
        )
    }
}

impl std::error::Error for ParseTypedIdError {}

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

        impl FromStr for $name {
            type Err = ParseTypedIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                let encoded = if let Some(encoded) = value.strip_prefix(concat!($prefix, "_")) {
                    encoded
                } else if value.contains('_') {
                    return Err(ParseTypedIdError {
                        value: value.to_owned(),
                        expected_prefix: $prefix,
                    });
                } else {
                    value
                };

                encoded
                    .parse::<Ulid>()
                    .map(Self)
                    .map_err(|_| ParseTypedIdError {
                        value: value.to_owned(),
                        expected_prefix: $prefix,
                    })
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

#[cfg(test)]
mod tests {
    use super::{AgentId, JobId};
    use std::str::FromStr;

    #[test]
    fn parses_prefixed_and_bare_ids() {
        let id = AgentId::new();
        let encoded = id.to_string();
        assert_eq!(AgentId::from_str(&encoded), Ok(id));
        assert_eq!(AgentId::from_str(&encoded[4..]), Ok(id));
    }

    #[test]
    fn rejects_another_resource_prefix() {
        let job = JobId::new();
        assert!(AgentId::from_str(&job.to_string()).is_err());
    }
}
