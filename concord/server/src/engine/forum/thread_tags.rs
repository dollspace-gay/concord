use super::{
    Actor, AuthService, AuthorizationError, AuthorizationService, ChannelAction, ForumError,
    ForumService, HashSet, Permissions, Row, ThreadTagMutation, insert_audit,
};

impl ForumService {
    pub async fn set_thread_tags(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        thread_id: &str,
        tag_ids: Vec<String>,
    ) -> Result<ThreadTagMutation, ForumError> {
        if tag_ids.len() > 5 || tag_ids.iter().collect::<HashSet<_>>().len() != tag_ids.len() {
            return Err(ForumError::Validation(
                "a thread may have up to 5 unique tags",
            ));
        }
        let (_permit, mut transaction) = self.writes.begin().await?;
        let authorization = AuthorizationService::new(self.pool.clone());
        let thread: (String, Option<String>, Option<String>, i64, i64, String) = sqlx::query_as(
            "SELECT channel_type,parent_channel_id,thread_creator_user_id, \
                        thread_tags_version,authorization_version,conversations.id \
                 FROM channels JOIN conversations ON conversations.channel_id=channels.id \
                 WHERE channels.id=? AND channels.server_id=?",
        )
        .bind(thread_id)
        .bind(server_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ForumError::Unavailable)?;
        if !matches!(thread.0.as_str(), "public_thread" | "private_thread") {
            return Err(ForumError::Unavailable);
        }
        let parent_channel_id = thread.1.ok_or(ForumError::Unavailable)?;
        authorization
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                &parent_channel_id,
                ChannelAction::View,
            )
            .await?;
        let can_manage = match authorization
            .require_channel_actor_permission_in(
                &mut transaction,
                auth,
                actor,
                server_id,
                &parent_channel_id,
                Permissions::MANAGE_CHANNELS,
            )
            .await
        {
            Ok(()) => true,
            Err(AuthorizationError::Unavailable) => false,
            Err(error) => return Err(error.into()),
        };
        if thread.2.as_deref() != Some(actor.user_id().as_str()) && !can_manage {
            return Err(ForumError::Unavailable);
        }
        let current: Vec<String> =
            sqlx::query_scalar("SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id")
                .bind(thread_id)
                .fetch_all(&mut *transaction)
                .await?;
        let changed = current
            .iter()
            .chain(tag_ids.iter())
            .filter(|id| current.contains(id) != tag_ids.contains(id))
            .cloned()
            .collect::<HashSet<_>>();
        let all = current
            .iter()
            .chain(tag_ids.iter())
            .cloned()
            .collect::<HashSet<_>>();
        if !all.is_empty() {
            let mut query =
                sqlx::QueryBuilder::new("SELECT id,moderated FROM forum_tags WHERE channel_id=");
            query.push_bind(&parent_channel_id).push(" AND id IN (");
            let mut values = query.separated(",");
            for id in &all {
                values.push_bind(id);
            }
            values.push_unseparated(")");
            let rows = query.build().fetch_all(&mut *transaction).await?;
            if rows.len() != all.len() {
                return Err(ForumError::Unavailable);
            }
            if !can_manage
                && rows.iter().any(|row| {
                    let id: String = row.get(0);
                    changed.contains(&id) && row.get::<i64, _>(1) != 0
                })
            {
                return Err(ForumError::Unavailable);
            }
        }
        sqlx::query("DELETE FROM thread_tags WHERE thread_id=?")
            .bind(thread_id)
            .execute(&mut *transaction)
            .await?;
        for tag_id in &tag_ids {
            sqlx::query("INSERT INTO thread_tags(thread_id,tag_id) VALUES(?,?)")
                .bind(thread_id)
                .bind(tag_id)
                .execute(&mut *transaction)
                .await?;
        }
        let version: i64 = sqlx::query_scalar(
            "UPDATE channels SET thread_tags_version=thread_tags_version+1 \
             WHERE id=? AND thread_tags_version=? RETURNING thread_tags_version",
        )
        .bind(thread_id)
        .bind(thread.3)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ForumError::Conflict)?;
        sqlx::query(
            "INSERT INTO entity_versions(entity_type,entity_id,version,updated_at) \
             VALUES('thread_tags',?,?,datetime('now')) \
             ON CONFLICT(entity_type,entity_id) DO UPDATE SET \
                version=excluded.version,updated_at=excluded.updated_at",
        )
        .bind(thread_id)
        .bind(version)
        .execute(&mut *transaction)
        .await?;
        let generation: String =
            sqlx::query_scalar("SELECT generation FROM database_metadata WHERE singleton=1")
                .fetch_one(&mut *transaction)
                .await?;
        let descriptor = serde_json::json!({"thread_id": thread_id, "tag_ids": tag_ids});
        let event_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO event_log( \
                database_generation,conversation_id,event_kind,entity_type,entity_id, \
                entity_version,authorization_version,actor_id,descriptor_json \
             ) VALUES(?,?,'thread_tags_updated','thread_tags',?,?,?,?,?) \
             RETURNING event_sequence",
        )
        .bind(generation)
        .bind(thread.5)
        .bind(thread_id)
        .bind(version)
        .bind(thread.4)
        .bind(actor.user_id().as_str())
        .bind(descriptor.to_string())
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query("INSERT INTO delivery_outbox(event_sequence) VALUES(?)")
            .bind(event_sequence)
            .execute(&mut *transaction)
            .await?;
        let changes = serde_json::json!({
            "old_tag_ids": current,
            "new_tag_ids": tag_ids,
            "version": version,
        })
        .to_string();
        insert_audit(
            &mut transaction,
            server_id,
            actor,
            "thread_tags_update",
            "thread",
            thread_id,
            Some(&changes),
        )
        .await?;
        transaction.commit().await?;
        Ok(ThreadTagMutation {
            thread_id: thread_id.to_string(),
            version,
            tag_ids,
        })
    }

    pub async fn get_thread_tags(
        &self,
        auth: &AuthService,
        actor: &Actor,
        server_id: &str,
        thread_id: &str,
    ) -> Result<(i64, Vec<String>), ForumError> {
        let mut transaction = self.pool.begin().await?;
        AuthorizationService::new(self.pool.clone())
            .authorize_actor_in(
                &mut transaction,
                auth,
                actor,
                thread_id,
                ChannelAction::View,
            )
            .await?;
        let version: i64 = sqlx::query_scalar(
            "SELECT thread_tags_version FROM channels WHERE id=? AND server_id=? \
             AND channel_type IN ('public_thread','private_thread')",
        )
        .bind(thread_id)
        .bind(server_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(ForumError::Unavailable)?;
        let tag_ids =
            sqlx::query_scalar("SELECT tag_id FROM thread_tags WHERE thread_id=? ORDER BY tag_id")
                .bind(thread_id)
                .fetch_all(&mut *transaction)
                .await?;
        transaction.commit().await?;
        Ok((version, tag_ids))
    }
}
