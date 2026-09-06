use super::{
    ChatEngine, ChatEvent, ConnectionId, HistoryMessage, ReactionGroup, ReplyInfo, Utc,
    normalize_channel_name, parse_persisted_timestamp, referenced_channel_id,
};
use crate::engine::validation;

impl ChatEngine {
    /// Set the topic for a channel.
    pub async fn set_topic(
        &self,
        session_id: ConnectionId,
        server_id: &str,
        channel_name: &str,
        topic: String,
    ) -> Result<(), String> {
        validation::validate_topic(&topic)?;
        let topic = validation::sanitize_html(&topic);
        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let session = self
            .sessions
            .get(&session_id)
            .ok_or("Session not found")?
            .clone();

        let channel = self
            .channels
            .get_mut(&channel_id)
            .ok_or(format!("No such channel: {channel_name}"))?;

        if !channel.members.contains(&session_id) {
            return Err(format!("You are not in channel {channel_name}"));
        }

        drop(channel);

        if let Some(pool) = &self.db {
            let actor = self
                .get_authenticated_actor(session_id)
                .ok_or_else(|| "resource unavailable".to_string())?;
            let auth = self.auth.get().ok_or("Authentication unavailable")?;
            crate::engine::organization::OrganizationService::new(
                pool.clone(),
                auth.clone(),
                self.write_admission
                    .as_ref()
                    .ok_or("Write admission unavailable")?
                    .clone(),
            )
            .set_topic(
                &actor,
                &referenced_channel_id(&channel_id)?,
                &topic,
                &session.nickname,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;
        }
        if let Some(mut channel) = self.channels.get_mut(&channel_id) {
            channel.topic.clone_from(&topic);
            channel.topic_set_by = Some(session.nickname.clone());
            channel.topic_set_at = Some(Utc::now());
        }

        let event = ChatEvent::TopicChange {
            server_id: server_id.to_string(),
            channel: channel_name,
            set_by: session.nickname.clone(),
            topic,
        };
        self.broadcast_to_channel(&channel_id, &event, None);

        Ok(())
    }
    /// Fetch message history for a channel, including edits, replies, and reactions.
    pub async fn fetch_history(
        &self,
        server_id: &str,
        channel_name: &str,
        before: Option<&str>,
        limit: i64,
        actor: &crate::auth::authority::Actor,
    ) -> Result<
        (
            Vec<HistoryMessage>,
            bool,
            crate::engine::authorization::AuthorizationStamp,
        ),
        String,
    > {
        let Some(pool) = &self.db else {
            return Err("No database configured".into());
        };

        let channel_name = normalize_channel_name(channel_name);
        let channel_id = self.resolve_channel_id(server_id, &channel_name)?;

        let auth = self.auth.get().ok_or("Authentication unavailable")?;
        let stamp = crate::engine::authorization::AuthorizationService::new(pool.clone())
            .authorize_actor_stamped(
                auth,
                actor,
                &channel_id,
                crate::engine::authorization::ChannelAction::ReadHistory,
            )
            .await
            .map_err(|_| "resource unavailable".to_string())?;

        let rows = crate::db::queries::messages::fetch_channel_history(
            pool,
            &channel_id,
            before,
            limit + 1,
        )
        .await
        .map_err(|e| format!("Failed to fetch history: {e}"))?;

        let has_more = rows.len() as i64 > limit;
        let rows: Vec<_> = rows.into_iter().take(limit as usize).collect();

        // Collect message IDs for batch reaction lookup
        let msg_ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();

        // Fetch reactions for all messages in batch
        let reaction_rows =
            crate::db::queries::messages::get_reactions_for_messages(pool, &msg_ids)
                .await
                .unwrap_or_default();

        // Group reactions by message_id -> emoji -> user_ids
        let mut reaction_map: std::collections::HashMap<
            String,
            std::collections::HashMap<String, Vec<String>>,
        > = std::collections::HashMap::new();
        for r in &reaction_rows {
            reaction_map
                .entry(r.message_id.clone())
                .or_default()
                .entry(r.emoji.clone())
                .or_default()
                .push(r.user_id.clone());
        }

        // Collect reply_to_ids for batch lookup
        let reply_ids: Vec<String> = rows.iter().filter_map(|r| r.reply_to_id.clone()).collect();
        let mut reply_map: std::collections::HashMap<String, ReplyInfo> =
            std::collections::HashMap::new();
        if !reply_ids.is_empty() {
            for rid in &reply_ids {
                if let Ok(Some((id, from, content))) = sqlx::query_as::<_, (String, String, String)>(
                    "SELECT id,sender_nick,CASE WHEN deleted_at IS NULL THEN content ELSE '' END FROM messages \
                     WHERE id=? AND conversation_id=(SELECT id FROM conversations WHERE channel_id=?)",
                )
                .bind(rid)
                .bind(&channel_id)
                .fetch_optional(pool)
                .await
                {
                    reply_map.insert(
                        id.clone(),
                        ReplyInfo {
                            id,
                            from,
                            content_preview: content.chars().take(100).collect(),
                        },
                    );
                }
            }
        }

        // Fetch attachments for all messages in batch
        let attachment_rows =
            crate::db::queries::attachments::get_attachments_for_messages(pool, &msg_ids)
                .await
                .unwrap_or_default();

        // Group attachments by message_id
        let mut attachment_map: std::collections::HashMap<
            String,
            Vec<crate::engine::events::AttachmentInfo>,
        > = std::collections::HashMap::new();
        for a in &attachment_rows {
            if let Some(ref mid) = a.message_id {
                attachment_map.entry(mid.clone()).or_default().push(
                    crate::engine::events::AttachmentInfo {
                        id: a.id.clone(),
                        filename: a.original_filename.clone(),
                        content_type: a.content_type.clone(),
                        file_size: a.file_size,
                        url: format!("/api/uploads/{}", a.id),
                    },
                );
            }
        }

        let mut rich_embed_map = std::collections::HashMap::new();
        let mut component_map = std::collections::HashMap::new();
        if !msg_ids.is_empty() {
            let mut builder = sqlx::QueryBuilder::new(
                "SELECT id,rich_embeds_json,components_json FROM messages WHERE id IN (",
            );
            let mut separated = builder.separated(",");
            for id in &msg_ids {
                separated.push_bind(id);
            }
            separated.push_unseparated(")");
            if let Ok(stored) = builder.build().fetch_all(pool).await {
                use sqlx::Row;
                for item in stored {
                    let id: String = item.get(0);
                    if let Some(value) = item.get::<Option<&str>, _>(1)
                        && let Ok(parsed) = serde_json::from_str(value)
                    {
                        rich_embed_map.insert(id.clone(), parsed);
                    }
                    if let Some(value) = item.get::<Option<&str>, _>(2)
                        && let Ok(parsed) = serde_json::from_str(value)
                    {
                        component_map.insert(id, parsed);
                    }
                }
            }
        }

        let messages: Vec<HistoryMessage> = rows
            .into_iter()
            .map(|row| -> Result<HistoryMessage, String> {
                let reactions = reaction_map.get(&row.id).map(|emoji_map| {
                    emoji_map
                        .iter()
                        .map(|(emoji, user_ids)| ReactionGroup {
                            emoji: emoji.clone(),
                            count: user_ids.len(),
                            user_ids: user_ids.clone(),
                        })
                        .collect()
                });
                let reply_to = row
                    .reply_to_id
                    .as_ref()
                    .and_then(|rid| reply_map.get(rid).cloned());
                let edited_at = row
                    .edited_at
                    .as_deref()
                    .map(|value| {
                        parse_persisted_timestamp(value).ok_or_else(|| {
                            "Stored message has an invalid edited timestamp".to_string()
                        })
                    })
                    .transpose()?;
                let timestamp = parse_persisted_timestamp(&row.created_at).ok_or_else(|| {
                    "Stored message has an invalid creation timestamp".to_string()
                })?;
                let attachments = attachment_map.remove(&row.id);
                let rich_embeds = rich_embed_map.remove(&row.id);
                let components = component_map.remove(&row.id);

                Ok(HistoryMessage {
                    id: crate::engine::ids::MessageId::from_stored(row.id)
                        .map_err(|_| "Stored message has an invalid identifier".to_string())?,
                    from: row.sender_nick,
                    content: row.content,
                    timestamp,
                    edited_at,
                    reply_to,
                    reactions,
                    attachments,
                    embeds: None,
                    rich_embeds,
                    components,
                })
            })
            .collect::<Result<_, _>>()?;

        Ok((messages, has_more, stamp))
    }
}
