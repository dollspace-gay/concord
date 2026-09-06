use std::borrow::Borrow;
use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::{Deserialize, Deserializer, Serialize};

const MAX_RESOURCE_ID_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceIdErrorKind {
    Empty,
    TooLong,
    SurroundingWhitespace,
    ControlCharacter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceIdError {
    resource: &'static str,
    kind: ResourceIdErrorKind,
}

impl ResourceIdError {
    #[must_use]
    pub fn resource(self) -> &'static str {
        self.resource
    }

    #[must_use]
    pub fn kind(self) -> ResourceIdErrorKind {
        self.kind
    }
}

impl fmt::Display for ResourceIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid {}", self.resource)
    }
}

impl std::error::Error for ResourceIdError {}

fn validate(value: &str, resource: &'static str) -> Result<(), ResourceIdError> {
    let kind = if value.is_empty() {
        Some(ResourceIdErrorKind::Empty)
    } else if value.len() > MAX_RESOURCE_ID_BYTES {
        Some(ResourceIdErrorKind::TooLong)
    } else if value.trim() != value {
        Some(ResourceIdErrorKind::SurroundingWhitespace)
    } else if value.chars().any(char::is_control) {
        Some(ResourceIdErrorKind::ControlCharacter)
    } else {
        None
    };
    match kind {
        Some(kind) => Err(ResourceIdError { resource, kind }),
        None => Ok(()),
    }
}

fn validate_stored(value: &str, resource: &'static str) -> Result<(), ResourceIdError> {
    let kind = if value.is_empty() {
        Some(ResourceIdErrorKind::Empty)
    } else if value.chars().any(char::is_control) {
        Some(ResourceIdErrorKind::ControlCharacter)
    } else {
        None
    };
    match kind {
        Some(kind) => Err(ResourceIdError { resource, kind }),
        None => Ok(()),
    }
}

macro_rules! resource_id {
    ($name:ident, $resource:literal, $stored_wire:literal) => {
        #[derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            Hash,
            PartialOrd,
            Ord,
            Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Validate an identifier entering through a current API boundary.
            pub fn parse(value: impl Into<String>) -> Result<Self, ResourceIdError> {
                let value = value.into();
                validate(&value, $resource)?;
                Ok(Self(value))
            }

            /// Validate a value already present in durable storage without
            /// imposing newer length or surrounding-whitespace restrictions.
            /// Callers retain the exact historical bytes.
            pub fn from_stored(value: impl Into<String>) -> Result<Self, ResourceIdError> {
                let value = value.into();
                validate_stored(&value, $resource)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl Borrow<str> for $name {
            fn borrow(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = ResourceIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = ResourceIdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl TryFrom<&str> for $name {
            type Error = ResourceIdError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                if $stored_wire {
                    Self::from_stored(value).map_err(serde::de::Error::custom)
                } else {
                    Self::parse(value).map_err(serde::de::Error::custom)
                }
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }

            fn json_schema(generator: &mut SchemaGenerator) -> Schema {
                let mut schema = generator.subschema_for::<String>();
                schema.insert("minLength".to_owned(), 1.into());
                if $stored_wire {
                    // Message events carry identifiers that may predate current
                    // issuance limits. JSON Schema string lengths count Unicode
                    // code points rather than UTF-8 bytes, so no maxLength can
                    // faithfully describe the byte limit used by `parse`.
                    schema.insert(
                        "pattern".to_owned(),
                        r"^[^\u0000-\u001F\u007F-\u009F]+$".into(),
                    );
                } else {
                    schema.insert(
                        "pattern".to_owned(),
                        r"^[^\s\u0000-\u001F\u007F-\u009F](?:[^\u0000-\u001F\u007F-\u009F]*[^\s\u0000-\u001F\u007F-\u009F])?$"
                            .into(),
                    );
                }
                schema
            }
        }
    };
}

resource_id!(ServerId, "server id", true);
resource_id!(ChannelId, "channel id", true);
resource_id!(ConversationId, "conversation id", true);
// Message IDs on the wire are references to already-stored rows. Their exact
// historical bytes must survive history, live events, replay and subsequent
// mutations. New IDs still enter through `parse` or the UUID conversion.
resource_id!(MessageId, "message id", true);

/// Process-local identifier for one live transport connection.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ConnectionId(uuid::Uuid);

impl ConnectionId {
    #[must_use]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    #[must_use]
    pub fn as_uuid(self) -> uuid::Uuid {
        self.0
    }
}

impl Default for ConnectionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl From<uuid::Uuid> for ConnectionId {
    fn from(value: uuid::Uuid) -> Self {
        Self(value)
    }
}

impl From<uuid::Uuid> for MessageId {
    fn from(value: uuid::Uuid) -> Self {
        Self(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_ids_preserve_legacy_non_uuid_values() {
        for value in ["legacy-server", "#channel:old/42", "did:plc:abc.def"] {
            let id = ServerId::parse(value).unwrap();
            assert_eq!(id.as_str(), value);
            assert_eq!(id.to_string(), value);
        }
    }

    #[test]
    fn stored_conversion_preserves_values_predating_current_input_limits() {
        let long = format!(" historical:{} ", "x".repeat(MAX_RESOURCE_ID_BYTES + 1));
        let id = ServerId::from_stored(long.clone()).unwrap();
        assert_eq!(id.as_str(), long);
        assert!(ServerId::parse(long).is_err());
    }

    #[test]
    fn every_resource_id_applies_the_same_input_boundary() {
        assert_eq!(
            ChannelId::parse("").unwrap_err().kind(),
            ResourceIdErrorKind::Empty
        );
        assert_eq!(
            ConversationId::parse(" padded").unwrap_err().kind(),
            ResourceIdErrorKind::SurroundingWhitespace
        );
        assert_eq!(
            MessageId::parse("line\nbreak").unwrap_err().kind(),
            ResourceIdErrorKind::ControlCharacter
        );
    }

    #[test]
    fn serde_round_trip_keeps_the_opaque_value() {
        let id = ChannelId::parse("historical:channel/7").unwrap();
        let encoded = serde_json::to_string(&id).unwrap();
        assert_eq!(encoded, "\"historical:channel/7\"");
        assert_eq!(serde_json::from_str::<ChannelId>(&encoded).unwrap(), id);
    }

    #[test]
    fn current_message_id_issuance_remains_strict() {
        for invalid in ["", " padded", "line\nbreak"] {
            assert!(MessageId::parse(invalid).is_err());
        }
        assert!(MessageId::parse("x".repeat(MAX_RESOURCE_ID_BYTES + 1)).is_err());
    }

    #[test]
    fn message_wire_deserialization_preserves_supported_historical_bytes() {
        for value in [
            "legacy:not-a-uuid".to_owned(),
            " historical message id ".to_owned(),
            format!(" 旧消息:{} ", "界".repeat(MAX_RESOURCE_ID_BYTES)),
        ] {
            let encoded = serde_json::to_string(&value).unwrap();
            let id = serde_json::from_str::<MessageId>(&encoded).unwrap();
            assert_eq!(id.as_str(), value);
            assert_eq!(serde_json::to_string(&id).unwrap(), encoded);
        }
        for invalid in ["\"\"", "\"line\\nbreak\"", "\"c1\\u0085control\""] {
            assert!(serde_json::from_str::<MessageId>(invalid).is_err());
        }
    }

    #[test]
    fn connection_ids_are_distinct_uuid_backed_values() {
        let first = ConnectionId::new();
        let second = ConnectionId::new();
        assert_ne!(first, second);
        assert_eq!(ConnectionId::from(first.as_uuid()), first);
        assert_eq!(
            serde_json::from_str::<ConnectionId>(&serde_json::to_string(&first).unwrap()).unwrap(),
            first
        );
    }

    #[test]
    fn message_wire_schema_accepts_supported_historical_values() {
        let schema = serde_json::to_value(schemars::schema_for!(MessageId)).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        for value in [
            serde_json::json!("legacy:not-a-uuid"),
            serde_json::json!(" historical message id "),
            serde_json::json!(format!(" 旧消息:{} ", "界".repeat(MAX_RESOURCE_ID_BYTES))),
        ] {
            assert!(validator.is_valid(&value), "schema rejected {value}");
        }
        for invalid in [
            serde_json::json!(""),
            serde_json::json!("line\nbreak"),
            serde_json::json!("c1\u{0085}control"),
        ] {
            assert!(!validator.is_valid(&invalid), "schema accepted {invalid}");
        }
        assert!(schema.get("maxLength").is_none());
    }
}
