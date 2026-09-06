use super::projection::event_to_irc_lines_inner;
use super::*;

use crate::engine::events::{MemberInfo, PinnedMessageInfo, ThreadInfo};

use crate::engine::ids::MessageId;

use chrono::Utc;

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use uuid::Uuid;

/// Create a minimal explicit in-memory harness for projection unit tests.
fn test_engine() -> Arc<ChatEngine> {
    Arc::new(ChatEngine::test_harness(4000, 100))
}

/// Test helper — calls the inner (tag-free) event formatter.
fn event_to_irc_lines(engine: &ChatEngine, my_nick: &str, event: &ChatEvent) -> Vec<String> {
    event_to_irc_lines_inner(engine, my_nick, event)
}

mod behavior;
mod identity;
mod lifecycle;
mod membership;
mod messaging;
mod queries;
