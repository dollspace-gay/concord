//! Machine-readable descriptions of Concord's current WebSocket wire contract.

use schemars::{JsonSchema, Schema, schema_for};

use crate::engine::events::ChatEvent;
use crate::web::ws_handler::ClientMessage;

pub const PROTOCOL_VERSION: u32 = 2;

/// The current bidirectional WebSocket payloads.
///
/// This is a generation root rather than a second wire model: both fields point
/// directly at the types serialized and deserialized by the production socket.
#[derive(JsonSchema)]
#[schemars(title = "Concord WebSocket contract")]
#[expect(
    dead_code,
    reason = "this type exists only as a JSON Schema generation root"
)]
pub struct WebSocketContract {
    client_message: ClientMessage,
    server_event: ChatEvent,
}

/// Generate JSON Schema from the production Serde DTO graph.
#[must_use]
pub fn websocket_schema() -> Schema {
    let mut schema = schema_for!(WebSocketContract);
    schema.insert(
        "x-concord-protocol-version".to_owned(),
        PROTOCOL_VERSION.into(),
    );
    schema
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ids::MessageId;
    use serde_json::json;

    #[test]
    fn schema_contains_both_wire_directions_and_discriminators() {
        let schema = serde_json::to_value(websocket_schema()).expect("schema must serialize");
        let rendered = schema.to_string();

        assert!(rendered.contains("client_message"));
        assert!(rendered.contains("server_event"));
        assert!(rendered.contains("send_message"));
        assert!(rendered.contains("message_ack"));
        assert!(rendered.contains("\"type\""));
        assert_eq!(schema["x-concord-protocol-version"], PROTOCOL_VERSION);
    }

    #[test]
    fn schema_accepts_actual_serde_values_and_rejects_wrong_payload_types() {
        let schema = serde_json::to_value(websocket_schema()).expect("schema must serialize");
        let validator = jsonschema::validator_for(&schema).expect("generated schema must compile");
        let client = json!({
            "type": "send_message",
            "operation_generation": "generation-0001",
            "server_id": "server-1",
            "channel": "general",
            "content": "hello",
            "reply_to": null,
            "attachment_ids": null,
            "nonce": "client-1"
        });
        let _: ClientMessage = serde_json::from_value(client.clone())
            .expect("fixture must use production deserializer");
        let server = serde_json::to_value(ChatEvent::Error {
            code: "INVALID_INPUT".into(),
            message: "invalid message".into(),
        })
        .expect("production event must serialize");

        assert!(validator.is_valid(&json!({
            "client_message": client,
            "server_event": server,
        })));
        assert!(!validator.is_valid(&json!({
            "client_message": {
                "type": "send_message",
                "server_id": "server-1",
                "channel": "general",
                "content": 7,
            },
            "server_event": {
                "type": "error",
                "code": "INVALID_INPUT",
                "message": "invalid message",
            },
        })));
        assert!(!validator.is_valid(&json!({
            "client_message": { "type": "unknown_command" },
            "server_event": { "type": "error", "code": 7, "message": false },
        })));
    }

    #[test]
    fn message_event_contract_preserves_historical_opaque_identifiers() {
        let schema = serde_json::to_value(websocket_schema()).expect("schema must serialize");
        let validator = jsonschema::validator_for(&schema).expect("generated schema must compile");
        for stored_id in [
            "legacy:not-a-uuid".to_owned(),
            " historical message id ".to_owned(),
            format!(" 旧消息:{} ", "界".repeat(512)),
        ] {
            let event = ChatEvent::Message {
                id: MessageId::from_stored(stored_id.clone()).expect("historical ID is supported"),
                server_id: Some("server-1".into()),
                conversation_id: None,
                from: "alice".into(),
                target: "#general".into(),
                content: "historical message".into(),
                timestamp: chrono::Utc::now(),
                avatar_url: None,
                reply_to: None,
                attachments: None,
            };
            let serialized = serde_json::to_value(&event).expect("event must serialize");
            let decoded: ChatEvent =
                serde_json::from_value(serialized.clone()).expect("event must deserialize");
            assert_eq!(
                serde_json::to_value(decoded).expect("decoded event must serialize"),
                serialized
            );
            assert_eq!(serialized["id"], stored_id);
            assert!(validator.is_valid(&json!({
                "client_message": {
                    "type": "list_servers"
                },
                "server_event": serialized,
            })));
        }
    }

    #[test]
    fn every_wire_variant_fixture_round_trips_through_production_serde() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../web/tests/contract-payloads.json"
        );
        let mut fixtures: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(fixture_path).expect("payload fixture corpus must exist"),
        )
        .expect("payload fixture corpus must be JSON");
        let schema = serde_json::to_value(websocket_schema()).expect("schema must serialize");
        let updating = std::env::var_os("UPDATE_CONTRACT_PAYLOAD_SNAPSHOTS").is_some();
        for direction in ["client_messages", "server_events"] {
            let cases = fixtures[direction]
                .as_array_mut()
                .expect("each fixture direction must be an array");
            for case in cases {
                for shape in ["minimal", "edge"] {
                    let payload = case[shape].clone();
                    let tag = payload["type"].as_str().expect("fixture must be tagged");
                    let round_trip = if direction == "client_messages" {
                        let decoded: ClientMessage = serde_json::from_value(payload.clone())
                            .unwrap_or_else(|error| {
                                panic!("{tag} {shape} client fixture: {error}")
                            });
                        serde_json::to_value(decoded).expect("client fixture must serialize")
                    } else {
                        let decoded: ChatEvent = serde_json::from_value(payload.clone())
                            .unwrap_or_else(|error| panic!("{tag} {shape} event fixture: {error}"));
                        serde_json::to_value(decoded).expect("event fixture must serialize")
                    };
                    let definition = if direction == "client_messages" {
                        "ClientMessage"
                    } else {
                        "ChatEvent"
                    };
                    let output_schema = json!({
                        "$ref": format!("#/$defs/{definition}"),
                        "$defs": schema["$defs"].clone(),
                    });
                    let validator = jsonschema::validator_for(&output_schema)
                        .expect("direction schema must compile");
                    assert!(
                        validator.is_valid(&round_trip),
                        "{tag} {shape} actual Rust serialization violates its schema"
                    );
                    let canonical_key = format!("canonical_{shape}");
                    if updating {
                        case[&canonical_key] = round_trip;
                    } else {
                        assert_eq!(
                            round_trip, case[&canonical_key],
                            "{tag} {shape} canonical null/default/skip serialization drifted"
                        );
                    }
                    let mut wrong_type = payload.clone();
                    wrong_type["type"] = json!("unknown_contract_variant");
                    assert!(
                        !validator.is_valid(&wrong_type),
                        "mutated {tag} type was accepted"
                    );
                }
            }
        }
        if updating {
            std::fs::write(
                fixture_path,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&fixtures).expect("fixtures serialize")
                ),
            )
            .expect("canonical payload snapshots must update");
        }
    }
}
