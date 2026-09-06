use super::*;

use uuid::Uuid;

fn roundtrip(event: &ChatEvent) -> ChatEvent {
    let json = serde_json::to_string(event).expect("serialize");
    serde_json::from_str(&json).expect("deserialize")
}

mod behavior;
mod identity;
mod lifecycle;
mod messaging;
mod queries;
mod validation;
