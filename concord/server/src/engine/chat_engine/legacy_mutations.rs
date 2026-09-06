use super::{ChatEngine, ChatEvent, ConnectionId, normalize_channel_name};
#[cfg(test)]
use super::{Permissions, Utc};
#[cfg(test)]
use crate::engine::validation;

impl ChatEngine {
    /// Edit a message's content. Only the sender or a moderator+ can edit.
    #[cfg(test)]
    pub async fn edit_message(
        &self,
        session_id: ConnectionId,
        message_id: &str,
        new_content: &str,
    ) -> Result<(), String> {
        validation::validate_message_with_limit(new_content, self.max_message_length)?;
        let new_content = &validation::sanitize_html(new_content);

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;
        // Only the sender can edit their own messages, unless user has MANAGE_MESSAGES
        let sender_id = session
            .user_id
            .as_deref()
            .ok_or("Authentication required to edit messages")?;
        if msg.sender_id != sender_id {
            let server_id = msg.server_id.as_deref().ok_or("Message has no server")?;
            let channel_id = msg.channel_id.as_deref().ok_or("Message has no channel")?;
            let perms = self
                .get_effective_permissions(server_id, Some(channel_id), sender_id)
                .await;
            if !perms.contains(Permissions::MANAGE_MESSAGES) {
                return Err("You can only edit your own messages".into());
            }
        }

        crate::db::queries::messages::update_message_content(pool, message_id, new_content)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        // Find the channel name for the event
        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::MessageEdit {
            id: crate::engine::ids::MessageId::from_stored(message_id)
                .map_err(|_| "Stored message has an invalid identifier".to_string())?,
            server_id: server_id.clone(),
            channel: channel_name,
            content: new_content.to_string(),
            edited_at: Utc::now(),
        };

        // Broadcast to the channel (including sender)
        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }
    /// Delete a message (soft delete). Sender can delete own, moderator+ can delete any.
    #[cfg(test)]
    pub async fn delete_message(
        &self,
        session_id: ConnectionId,
        message_id: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;

        let sender_id = session
            .user_id
            .as_deref()
            .ok_or("Authentication required to delete messages")?;
        let is_sender = msg.sender_id == sender_id;

        if !is_sender {
            // Check if user has MANAGE_MESSAGES permission
            let server_id = msg.server_id.as_deref().ok_or("Message has no server")?;
            let channel_id_ref = msg.channel_id.as_deref().ok_or("Message has no channel")?;
            let perms = self
                .get_effective_permissions(server_id, Some(channel_id_ref), sender_id)
                .await;
            if !perms.contains(Permissions::MANAGE_MESSAGES) {
                return Err("You can only delete your own messages".into());
            }
        }

        crate::db::queries::messages::soft_delete_message(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::MessageDelete {
            id: crate::engine::ids::MessageId::from_stored(message_id)
                .map_err(|_| "Stored message has an invalid identifier".to_string())?,
            server_id,
            channel: channel_name,
        };

        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }
    /// Add a reaction to a message.
    #[cfg(test)]
    pub async fn add_reaction(
        &self,
        session_id: ConnectionId,
        message_id: &str,
        emoji: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;
        let user_id = session.user_id.as_deref().unwrap_or(&session.nickname);

        crate::db::queries::messages::add_reaction(pool, message_id, user_id, emoji)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::ReactionAdd {
            message_id: crate::engine::ids::MessageId::from_stored(message_id)
                .map_err(|_| "Stored message has an invalid identifier".to_string())?,
            server_id,
            channel: channel_name,
            user_id: user_id.to_string(),
            nickname: session.nickname.clone(),
            emoji: emoji.to_string(),
        };

        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }
    /// Remove a reaction from a message.
    #[cfg(test)]
    pub async fn remove_reaction(
        &self,
        session_id: ConnectionId,
        message_id: &str,
        emoji: &str,
    ) -> Result<(), String> {
        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let pool = self.db.as_ref().ok_or("No database configured")?;

        let msg = crate::db::queries::messages::get_message_by_id(pool, message_id)
            .await
            .map_err(|e| format!("DB error: {e}"))?
            .ok_or("Message not found")?;

        let user_id = session.user_id.as_deref().unwrap_or(&session.nickname);

        crate::db::queries::messages::remove_reaction(pool, message_id, user_id, emoji)
            .await
            .map_err(|e| format!("DB error: {e}"))?;

        let server_id = msg.server_id.ok_or("Message has no server")?;
        let channel_id = msg.channel_id.ok_or("Message has no channel")?;

        let channel_name = self
            .channels
            .get(&channel_id)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        let event = ChatEvent::ReactionRemove {
            message_id: crate::engine::ids::MessageId::from_stored(message_id)
                .map_err(|_| "Stored message has an invalid identifier".to_string())?,
            server_id,
            channel: channel_name,
            user_id: user_id.to_string(),
            nickname: session.nickname.clone(),
            emoji: emoji.to_string(),
        };

        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }
    /// Broadcast a typing indicator to a channel.
    pub fn send_typing(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
    ) -> Result<(), String> {
        let channel_name = normalize_channel_name(channel_name);

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let event = ChatEvent::TypingStart {
            server_id: server_id.to_string(),
            channel: channel_name,
            nickname: session.nickname.clone(),
        };

        self.broadcast_to_channel(&channel_id, &event, Some(session_id));

        Ok(())
    }
}
