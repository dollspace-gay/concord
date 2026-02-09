use std::sync::Arc;

use chrono::Utc;
use dashmap::DashMap;
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use super::channel::ChannelState;
use super::events::{ChannelInfo, ChatEvent, SessionId};
use super::user_session::{Protocol, UserSession};

/// The central hub that manages all chat state. Protocol-agnostic —
/// both IRC and WebSocket adapters call into this.
pub struct ChatEngine {
    /// All currently connected sessions, keyed by session ID.
    sessions: DashMap<SessionId, Arc<UserSession>>,
    /// All channels, keyed by channel name (e.g. "#general").
    channels: DashMap<String, ChannelState>,
    /// Reverse lookup: nickname -> session ID (for DMs and WHOIS).
    nick_to_session: DashMap<String, SessionId>,
}

impl ChatEngine {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            channels: DashMap::new(),
            nick_to_session: DashMap::new(),
        }
    }

    /// Register a new session. Returns the session ID and an event receiver
    /// that the protocol adapter should read from for outbound events.
    pub fn connect(
        &self,
        nickname: String,
        protocol: Protocol,
    ) -> Result<(SessionId, mpsc::UnboundedReceiver<ChatEvent>), String> {
        // Check nickname availability
        if self.nick_to_session.contains_key(&nickname) {
            return Err(format!("Nickname '{}' is already in use", nickname));
        }

        let session_id = Uuid::new_v4();
        let (tx, rx) = mpsc::unbounded_channel();

        let session = Arc::new(UserSession::new(
            session_id,
            nickname.clone(),
            protocol,
            tx,
        ));

        self.sessions.insert(session_id, session);
        self.nick_to_session.insert(nickname.clone(), session_id);

        info!(%session_id, %nickname, ?protocol, "session connected");

        Ok((session_id, rx))
    }

    /// Disconnect a session and clean up all state.
    pub fn disconnect(&self, session_id: SessionId) {
        let Some((_, session)) = self.sessions.remove(&session_id) else {
            return;
        };

        let nickname = session.nickname.clone();
        self.nick_to_session.remove(&nickname);

        // Remove from all channels and notify remaining members
        let channels_to_leave: Vec<String> = session.channels.iter().cloned().collect();
        for channel_name in &channels_to_leave {
            if let Some(mut channel) = self.channels.get_mut(channel_name) {
                channel.members.remove(&session_id);
            }
        }

        // Broadcast quit to all channels this user was in
        let quit_event = ChatEvent::Quit {
            nickname: nickname.clone(),
            reason: None,
        };

        for channel_name in &channels_to_leave {
            self.broadcast_to_channel(channel_name, &quit_event, Some(session_id));
        }

        info!(%session_id, %nickname, "session disconnected");
    }

    /// Join a channel. Creates the channel if it doesn't exist.
    pub fn join_channel(&self, session_id: SessionId, channel_name: &str) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);

        // Get session (need to update its channel set)
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        // Create channel if it doesn't exist
        self.channels
            .entry(channel_name.clone())
            .or_insert_with(|| ChannelState::new(channel_name.clone()));

        // Add session to channel
        if let Some(mut channel) = self.channels.get_mut(&channel_name) {
            channel.members.insert(session_id);
        }

        // We can't mutate through Arc, so we track channel membership in the
        // ChannelState. The session.channels field is set at creation and we
        // won't mutate it here — the channel's member set is the source of truth.

        // Broadcast join event to the channel (including the joiner)
        let join_event = ChatEvent::Join {
            nickname: session.nickname.clone(),
            channel: channel_name.clone(),
        };
        self.broadcast_to_channel(&channel_name, &join_event, None);

        // Send current topic to the joiner
        if let Some(channel) = self.channels.get(&channel_name) {
            if !channel.topic.is_empty() {
                let _ = session.send(ChatEvent::Topic {
                    channel: channel_name.clone(),
                    topic: channel.topic.clone(),
                });
            }

            // Send member list to the joiner
            let members: Vec<String> = channel
                .members
                .iter()
                .filter_map(|sid| self.sessions.get(sid).map(|s| s.nickname.clone()))
                .collect();

            let _ = session.send(ChatEvent::Names {
                channel: channel_name.clone(),
                members,
            });
        }

        info!(nickname = %session.nickname, %channel_name, "joined channel");
        Ok(())
    }

    /// Leave a channel.
    pub fn part_channel(
        &self,
        session_id: SessionId,
        channel_name: &str,
        reason: Option<String>,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let mut found = false;
        if let Some(mut channel) = self.channels.get_mut(&channel_name) {
            found = channel.members.remove(&session_id);
        }

        if !found {
            return Err(format!("Not in channel {}", channel_name));
        }

        // Broadcast part event
        let part_event = ChatEvent::Part {
            nickname: session.nickname.clone(),
            channel: channel_name.clone(),
            reason,
        };
        // Notify remaining members AND the departing user
        let _ = session.send(part_event.clone());
        self.broadcast_to_channel(&channel_name, &part_event, Some(session_id));

        // Remove empty channels
        self.channels
            .remove_if(&channel_name, |_, ch| ch.members.is_empty());

        info!(nickname = %session.nickname, %channel_name, "parted channel");
        Ok(())
    }

    /// Send a message to a channel or user (DM).
    pub fn send_message(
        &self,
        session_id: SessionId,
        target: &str,
        content: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let event = ChatEvent::Message {
            id: Uuid::new_v4(),
            from: session.nickname.clone(),
            target: target.to_string(),
            content: content.to_string(),
            timestamp: Utc::now(),
        };

        if target.starts_with('#') {
            // Channel message
            let channel_name = normalize_channel_name(target);
            let channel = self
                .channels
                .get(&channel_name)
                .ok_or(format!("No such channel: {}", channel_name))?;

            if !channel.members.contains(&session_id) {
                return Err(format!("You are not in channel {}", channel_name));
            }

            // Broadcast to all members except the sender
            self.broadcast_to_channel(&channel_name, &event, Some(session_id));
        } else {
            // DM — find the target user by nickname
            let target_session_id = self
                .nick_to_session
                .get(target)
                .ok_or(format!("No such user: {}", target))?;

            if let Some(target_session) = self.sessions.get(target_session_id.value()) {
                let _ = target_session.send(event);
            }
        }

        Ok(())
    }

    /// Set the topic for a channel.
    pub fn set_topic(
        &self,
        session_id: SessionId,
        channel_name: &str,
        topic: String,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let mut channel = self
            .channels
            .get_mut(&channel_name)
            .ok_or(format!("No such channel: {}", channel_name))?;

        if !channel.members.contains(&session_id) {
            return Err(format!("You are not in channel {}", channel_name));
        }

        channel.topic = topic.clone();
        channel.topic_set_by = Some(session.nickname.clone());
        channel.topic_set_at = Some(Utc::now());

        drop(channel);

        let event = ChatEvent::TopicChange {
            channel: channel_name.clone(),
            set_by: session.nickname.clone(),
            topic,
        };
        self.broadcast_to_channel(&channel_name, &event, None);

        Ok(())
    }

    /// List all channels.
    pub fn list_channels(&self) -> Vec<ChannelInfo> {
        self.channels
            .iter()
            .map(|entry| ChannelInfo {
                name: entry.name.clone(),
                topic: entry.topic.clone(),
                member_count: entry.member_count(),
            })
            .collect()
    }

    /// Get members of a channel.
    pub fn get_members(&self, channel_name: &str) -> Result<Vec<String>, String> {
        let channel_name = normalize_channel_name(channel_name);
        let channel = self
            .channels
            .get(&channel_name)
            .ok_or(format!("No such channel: {}", channel_name))?;

        Ok(channel
            .members
            .iter()
            .filter_map(|sid| self.sessions.get(sid).map(|s| s.nickname.clone()))
            .collect())
    }

    /// Check if a nickname is available.
    pub fn is_nick_available(&self, nickname: &str) -> bool {
        !self.nick_to_session.contains_key(nickname)
    }

    /// Get a session by ID (used by protocol adapters to send direct responses).
    pub fn get_session(&self, session_id: SessionId) -> Option<Arc<UserSession>> {
        self.sessions.get(&session_id).map(|s| s.clone())
    }

    /// Broadcast an event to all members of a channel, optionally excluding one session.
    fn broadcast_to_channel(
        &self,
        channel_name: &str,
        event: &ChatEvent,
        exclude: Option<SessionId>,
    ) {
        let Some(channel) = self.channels.get(channel_name) else {
            return;
        };

        for member_id in &channel.members {
            if Some(*member_id) == exclude {
                continue;
            }
            if let Some(session) = self.sessions.get(member_id) {
                if !session.send(event.clone()) {
                    warn!(%member_id, "failed to send event to session (channel closed)");
                }
            }
        }
    }
}

/// Ensure channel names are lowercase and start with #.
fn normalize_channel_name(name: &str) -> String {
    let name = name.to_lowercase();
    if name.starts_with('#') {
        name
    } else {
        format!("#{}", name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_channel_name() {
        assert_eq!(normalize_channel_name("#General"), "#general");
        assert_eq!(normalize_channel_name("general"), "#general");
        assert_eq!(normalize_channel_name("#rust"), "#rust");
    }

    #[tokio::test]
    async fn test_connect_and_disconnect() {
        let engine = ChatEngine::new();

        let (session_id, _rx) = engine.connect("alice".into(), Protocol::WebSocket).unwrap();
        assert!(!engine.is_nick_available("alice"));

        engine.disconnect(session_id);
        assert!(engine.is_nick_available("alice"));
    }

    #[tokio::test]
    async fn test_duplicate_nick_rejected() {
        let engine = ChatEngine::new();

        let (_sid, _rx) = engine.connect("alice".into(), Protocol::WebSocket).unwrap();
        let result = engine.connect("alice".into(), Protocol::WebSocket);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_join_and_message() {
        let engine = ChatEngine::new();

        let (sid1, mut rx1) = engine.connect("alice".into(), Protocol::WebSocket).unwrap();
        let (sid2, mut rx2) = engine.connect("bob".into(), Protocol::WebSocket).unwrap();

        engine.join_channel(sid1, "#general").unwrap();
        engine.join_channel(sid2, "#general").unwrap();

        // Drain join/names events
        while rx1.try_recv().is_ok() {}
        while rx2.try_recv().is_ok() {}

        engine
            .send_message(sid1, "#general", "Hello from Alice!")
            .unwrap();

        // Bob should receive the message, Alice should not (sender excluded)
        let event = rx2.try_recv().unwrap();
        match event {
            ChatEvent::Message {
                from, content, ..
            } => {
                assert_eq!(from, "alice");
                assert_eq!(content, "Hello from Alice!");
            }
            _ => panic!("Expected Message event, got {:?}", event),
        }

        assert!(rx1.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_part_channel() {
        let engine = ChatEngine::new();

        let (sid1, mut rx1) = engine.connect("alice".into(), Protocol::WebSocket).unwrap();
        let (sid2, _rx2) = engine.connect("bob".into(), Protocol::WebSocket).unwrap();

        engine.join_channel(sid1, "#general").unwrap();
        engine.join_channel(sid2, "#general").unwrap();

        while rx1.try_recv().is_ok() {}

        engine.part_channel(sid2, "#general", None).unwrap();

        // Alice should get a Part event
        let event = rx1.try_recv().unwrap();
        match event {
            ChatEvent::Part { nickname, .. } => assert_eq!(nickname, "bob"),
            _ => panic!("Expected Part event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_set_topic() {
        let engine = ChatEngine::new();

        let (sid, mut rx) = engine.connect("alice".into(), Protocol::WebSocket).unwrap();
        engine.join_channel(sid, "#general").unwrap();
        while rx.try_recv().is_ok() {}

        engine
            .set_topic(sid, "#general", "Welcome to Concord!".into())
            .unwrap();

        let event = rx.try_recv().unwrap();
        match event {
            ChatEvent::TopicChange { topic, .. } => {
                assert_eq!(topic, "Welcome to Concord!");
            }
            _ => panic!("Expected TopicChange event, got {:?}", event),
        }
    }

    #[tokio::test]
    async fn test_dm() {
        let engine = ChatEngine::new();

        let (sid1, _rx1) = engine.connect("alice".into(), Protocol::WebSocket).unwrap();
        let (_sid2, mut rx2) = engine.connect("bob".into(), Protocol::WebSocket).unwrap();

        engine.send_message(sid1, "bob", "Hey Bob!").unwrap();

        let event = rx2.try_recv().unwrap();
        match event {
            ChatEvent::Message {
                from,
                target,
                content,
                ..
            } => {
                assert_eq!(from, "alice");
                assert_eq!(target, "bob");
                assert_eq!(content, "Hey Bob!");
            }
            _ => panic!("Expected Message event, got {:?}", event),
        }
    }

    #[test]
    fn test_list_channels() {
        let engine = ChatEngine::new();

        let (sid, _rx) = engine.connect("alice".into(), Protocol::WebSocket).unwrap();
        engine.join_channel(sid, "#general").unwrap();
        engine.join_channel(sid, "#rust").unwrap();

        let channels = engine.list_channels();
        assert_eq!(channels.len(), 2);

        let names: Vec<&str> = channels.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"#general"));
        assert!(names.contains(&"#rust"));
    }
}
