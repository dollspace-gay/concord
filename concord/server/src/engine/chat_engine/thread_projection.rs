use super::{ChatEngine, ChatEvent, ThreadInfo};

impl ChatEngine {
    pub(super) async fn insert_thread_state_event_in(
        connection: &mut sqlx::SqliteConnection,
        thread_id: &str,
        version: i64,
        archived: bool,
        reason: Option<&str>,
        actor_id: &str,
    ) -> Result<(), String> {
        let scope: Option<(String, i64)> = sqlx::query_as(
            "SELECT cv.id,c.authorization_version FROM channels c \
             JOIN conversations cv ON cv.channel_id=c.id WHERE c.id=?",
        )
        .bind(thread_id)
        .fetch_optional(&mut *connection)
        .await
        .map_err(|error| error.to_string())?;
        let (conversation_id, authorization_version) =
            scope.ok_or_else(|| "thread conversation unavailable".to_string())?;
        sqlx::query(
            "INSERT INTO entity_versions(entity_type,entity_id,version,updated_at) \
             VALUES('thread_state',?,?,datetime('now')) \
             ON CONFLICT(entity_type,entity_id) DO UPDATE SET \
                version=excluded.version,updated_at=excluded.updated_at",
        )
        .bind(thread_id)
        .bind(version)
        .execute(&mut *connection)
        .await
        .map_err(|error| error.to_string())?;
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *connection)
                .await
                .map_err(|error| error.to_string())?;
        let event_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO event_log( \
                database_generation,conversation_id,event_kind,entity_type,entity_id, \
                entity_version,authorization_version,actor_id,descriptor_json \
             ) VALUES(?,?,'thread_state_changed','thread_state',?,?,?,?,?) \
             RETURNING event_sequence",
        )
        .bind(generation)
        .bind(conversation_id)
        .bind(thread_id)
        .bind(version)
        .bind(authorization_version)
        .bind(actor_id)
        .bind(serde_json::json!({"archived": archived, "reason": reason}).to_string())
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| error.to_string())?;
        sqlx::query("INSERT INTO delivery_outbox(event_sequence) VALUES(?)")
            .bind(event_sequence)
            .execute(connection)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    pub(super) async fn project_thread_state(&self, thread_id: &str) -> Result<(), String> {
        let pool = self.db.as_ref().ok_or("No database configured")?;
        let persisted: Option<(i64, i64)> =
            sqlx::query_as("SELECT archived,thread_state_version FROM channels WHERE id=?")
                .bind(thread_id)
                .fetch_optional(pool)
                .await
                .map_err(|error| error.to_string())?;
        let Some((archived, state_version)) = persisted else {
            return Ok(());
        };
        self.apply_thread_state_projection(thread_id, archived != 0, state_version);
        Ok(())
    }
    pub(super) fn apply_thread_state_projection(
        &self,
        thread_id: &str,
        archived: bool,
        state_version: i64,
    ) {
        let Some(mut channel) = self.channels.get_mut(thread_id) else {
            return;
        };
        if state_version < channel.thread_state_version {
            return;
        }
        channel.archived = archived;
        channel.thread_state_version = state_version;
        let event = ChatEvent::ThreadUpdate {
            server_id: channel.server_id.clone(),
            thread: ThreadInfo {
                id: channel.id.clone(),
                name: channel.name.clone(),
                channel_type: channel.channel_type.clone(),
                parent_message_id: channel.thread_parent_message_id.clone(),
                creator_user_id: channel.thread_creator_user_id.clone(),
                archived,
                state_version,
                tags_version: channel.thread_tags_version,
                tag_ids: channel.thread_tag_ids.clone(),
                auto_archive_minutes: channel.auto_archive_minutes,
                message_count: 0,
                created_at: channel.created_at.to_rfc3339(),
            },
        };
        drop(channel);
        self.broadcast_to_channel(thread_id, &event, None);
    }
}
